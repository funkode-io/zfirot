use serde::{Deserialize, Serialize};

use crate::PrdRef;

/// The derived state of a [`Slice`].
///
/// Precedence among active states is Blocked > WIP > Ready. `Done` (a closed
/// Slice) is a real state too, so the derivation is total; the board simply
/// omits it from its columns ([`SliceState::BOARD`]), keeping the data around so
/// Done Slices can be shown later if needed. The state is a pure derivation over
/// current GitHub data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SliceState {
    /// Blockers all closed, no open linked PR, and no assignee.
    Ready,
    /// An open Pull Request is linked to the Slice, or an assignee has claimed
    /// it.
    Wip,
    /// At least one open "blocked by" dependency.
    Blocked,
    /// A closed Slice. Hidden from the active board.
    Done,
}

impl SliceState {
    /// Board column order, left to right. `Done` is intentionally excluded so
    /// closed Slices are hidden from the active board.
    pub const BOARD: [SliceState; 3] = [SliceState::Ready, SliceState::Wip, SliceState::Blocked];
}

/// At-a-glance counts of the active board's Slices by state, for the summary
/// strip above the columns.
///
/// Only the three board states are counted; `Done` (closed) Slices are hidden
/// from the active board, so they are excluded from every count — `total` is
/// therefore `ready + wip + blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BoardSummary {
    pub ready: usize,
    pub wip: usize,
    pub blocked: usize,
}

impl BoardSummary {
    /// Count the Slices in each board state, ignoring `Done` Slices.
    ///
    /// A pure derivation over the Slices' already-derived [`SliceState`], so it
    /// stays consistent with what the columns show.
    pub fn from_slices<'a>(slices: impl IntoIterator<Item = &'a Slice>) -> Self {
        let mut summary = BoardSummary::default();
        for slice in slices {
            match slice.state {
                SliceState::Ready => summary.ready += 1,
                SliceState::Wip => summary.wip += 1,
                SliceState::Blocked => summary.blocked += 1,
                SliceState::Done => {}
            }
        }
        summary
    }

    /// The number of Slices shown on the active board (Ready + WIP + Blocked).
    pub fn total(&self) -> usize {
        self.ready + self.wip + self.blocked
    }
}

/// A reference to a related issue, for rendering a clickable dependency badge on
/// a card: either a **blocker** (an issue this Slice is blocked by) or an issue
/// this Slice **unblocks** (the reverse edge). Carries the issue number (shown
/// on the badge), its title (shown as a tooltip), and its URL (the badge links
/// to it on GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRef {
    /// The referenced GitHub issue number.
    pub number: u64,
    /// The referenced issue's title, shown as the badge tooltip.
    pub title: String,
    /// The referenced issue's URL on GitHub, for opening it in a browser.
    pub url: String,
}

/// A reference to an open Pull Request that closes a Slice's issue (GitHub's
/// closing reference), for rendering a clickable `pr #n @u` badge on a card.
/// Carries the PR number (shown on the badge), its author's login (shown as the
/// `@u` segment, absent when GitHub cannot resolve an author), its title (shown
/// as a tooltip), and its URL (the badge links to it on GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedPrRef {
    /// The referenced GitHub Pull Request number.
    pub number: u64,
    /// The PR author's login, shown as the `@u` segment; `None` when GitHub
    /// cannot resolve an author (e.g. a deleted account), in which case the
    /// badge omits the `@u` segment.
    pub author: Option<String>,
    /// The PR's title, shown as the badge tooltip.
    pub title: String,
    /// The PR's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// The review-lifecycle stage of this PR (Draft ... Approved), derived from
    /// GitHub's draft flag and review decision. Drives the Slice's WIP headline
    /// (via its Best PR); merge-health Decorations ride on top of it.
    pub pr_status: crate::PrStatus,
    /// `true` when the PR conflicts with its base branch and needs a manual
    /// conflict merge (GitHub `mergeable = CONFLICTING`). A branch merely behind
    /// its base (auto-updatable) is deliberately not flagged.
    pub conflicts: bool,
}

/// A read model of a GitHub issue that is a Slice of a PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// The PRD this Slice belongs to, when known.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// Avatar URL of the assignee, when assigned and available.
    pub assignee_avatar_url: Option<String>,
    pub state: SliceState,
    /// The still-open issues this Slice is blocked by, for the blocker badges.
    pub blockers: Vec<DependencyRef>,
    /// The issues this Slice unblocks (the reverse "blocked by" edge), for the
    /// unblocks badges. Derived across the board by [`resolve_unblocks`].
    pub unblocks: Vec<DependencyRef>,
    /// The open Pull Requests linked to this Slice via their closing reference,
    /// for the `pr #n @u` badges.
    pub linked_prs: Vec<LinkedPrRef>,
}

/// Raw, GitHub-shaped facts about a single issue, before its [`SliceState`] is
/// derived. An adapter projects this from GitHub (still fake for this slice);
/// the pure derivation lives in the domain so it stays testable and offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSlice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// `true` when the issue is closed; a closed Slice is Done and hidden.
    pub closed: bool,
    /// The PRD this Slice belongs to, when known.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// Avatar URL of the assignee, when assigned and available.
    pub assignee_avatar_url: Option<String>,
    /// The open Pull Requests linked to the issue via their closing reference.
    /// A non-empty list makes the Slice WIP.
    pub linked_prs: Vec<LinkedPrRef>,
    /// The still-open "blocked by" dependencies, with their references. A
    /// non-empty list makes the Slice Blocked.
    pub blockers: Vec<DependencyRef>,
    /// The issues this Slice unblocks, derived across the board by
    /// [`resolve_unblocks`]; empty until then.
    pub unblocks: Vec<DependencyRef>,
}

impl RawSlice {
    /// Project this raw issue into a [`Slice`] with its derived [`SliceState`].
    pub fn into_slice(self) -> Slice {
        let state = self.derive_state();
        Slice {
            number: self.number,
            title: self.title,
            url: self.url,
            prd: self.prd,
            assignee: self.assignee,
            assignee_avatar_url: self.assignee_avatar_url,
            state,
            blockers: self.blockers,
            unblocks: self.unblocks,
            linked_prs: self.linked_prs,
        }
    }

    /// The pure `SliceState` derivation.
    ///
    /// A closed Slice is always `Done`. Otherwise precedence is
    /// Blocked > WIP > Ready:
    /// - **Blocked**: at least one open "blocked by" dependency.
    /// - **WIP**: an open linked PR, or an assignee has claimed it to start work
    ///   (an assigned Slice is by definition no longer Ready).
    /// - **Ready**: all blockers closed, no open linked PR, and no assignee.
    fn derive_state(&self) -> SliceState {
        if self.closed {
            return SliceState::Done;
        }
        if !self.blockers.is_empty() {
            return SliceState::Blocked;
        }
        if !self.linked_prs.is_empty() || self.assignee.is_some() {
            return SliceState::Wip;
        }
        SliceState::Ready
    }
}

/// Derive each Slice's reverse **"unblocks"** edge from the blocker edges across
/// the whole board: a Slice unblocks every Slice that lists it as a blocker.
///
/// Pure and order-preserving — a Slice's `unblocks` list follows board input
/// order. Only fetched Slices contribute edges, so references to issues outside
/// the fetched set (e.g. closed or absent blockers) are naturally omitted.
pub fn resolve_unblocks(slices: &mut [RawSlice]) {
    use std::collections::HashMap;

    // For each blocker issue number, the references of the Slices it unblocks
    // (i.e. the Slices that listed it as a blocker), in board order.
    let mut unblocks_by_number: HashMap<u64, Vec<DependencyRef>> = HashMap::new();
    for slice in slices.iter() {
        let dependent = DependencyRef {
            number: slice.number,
            title: slice.title.clone(),
            url: slice.url.clone(),
        };
        for blocker in &slice.blockers {
            unblocks_by_number
                .entry(blocker.number)
                .or_default()
                .push(dependent.clone());
        }
    }
    for slice in slices.iter_mut() {
        // Always reset from the current blocker edges (defaulting to empty) so
        // the derivation is pure: re-running it never leaves stale reverse-edge
        // data on a Slice that no longer unblocks anything.
        slice.unblocks = unblocks_by_number.remove(&slice.number).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw Slice with no blockers, no PR, and no assignee (a Ready baseline).
    fn ready_raw() -> RawSlice {
        RawSlice {
            number: 1,
            title: "A Slice".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/1".to_string(),
            closed: false,
            prd: Some(PrdRef {
                number: 7,
                title: "A PRD".to_string(),
                url: "https://github.com/funkode-io/zfirot/issues/7".to_string(),
            }),
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            blockers: vec![],
            unblocks: vec![],
        }
    }

    /// A single open linked PR reference, for exercising WIP derivation and
    /// carry-through.
    fn linked_pr() -> LinkedPrRef {
        LinkedPrRef {
            number: 200,
            author: Some("hubot".to_string()),
            title: "Implement the Slice".to_string(),
            url: "https://github.com/funkode-io/zfirot/pull/200".to_string(),
            pr_status: crate::PrStatus::AwaitingReview,
            conflicts: false,
        }
    }

    /// `n` distinct open blocker references, for exercising state derivation.
    fn blockers(n: u64) -> Vec<DependencyRef> {
        (0..n)
            .map(|i| DependencyRef {
                number: 100 + i,
                title: format!("Blocker {}", 100 + i),
                url: format!("https://github.com/funkode-io/zfirot/issues/{}", 100 + i),
            })
            .collect()
    }

    #[test]
    fn derives_each_state_including_done() {
        struct Case {
            name: &'static str,
            closed: bool,
            assignee: Option<&'static str>,
            has_open_linked_pr: bool,
            open_blocker_count: u64,
            expected: SliceState,
        }
        let cases = [
            Case {
                name: "no blockers, no PR, no assignee -> Ready",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: SliceState::Ready,
            },
            Case {
                name: "open linked PR -> WIP",
                closed: false,
                assignee: None,
                has_open_linked_pr: true,
                open_blocker_count: 0,
                expected: SliceState::Wip,
            },
            Case {
                name: "assigned but no PR -> WIP (no longer Ready)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: SliceState::Wip,
            },
            Case {
                name: "open blocker -> Blocked",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 1,
                expected: SliceState::Blocked,
            },
            Case {
                name: "Blocked outranks WIP (PR + open blocker)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: true,
                open_blocker_count: 2,
                expected: SliceState::Blocked,
            },
            Case {
                name: "WIP outranks Ready (assignee present)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: SliceState::Wip,
            },
            Case {
                name: "closed -> Done",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: SliceState::Done,
            },
            Case {
                name: "closed wins even with an open blocker",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 3,
                expected: SliceState::Done,
            },
        ];

        for case in cases {
            let raw = RawSlice {
                closed: case.closed,
                assignee: case.assignee.map(str::to_string),
                linked_prs: if case.has_open_linked_pr {
                    vec![linked_pr()]
                } else {
                    vec![]
                },
                blockers: blockers(case.open_blocker_count),
                ..ready_raw()
            };

            assert_eq!(raw.derive_state(), case.expected, "{}", case.name);
        }
    }

    #[test]
    fn done_is_excluded_from_the_board_columns() {
        assert!(
            !SliceState::BOARD.contains(&SliceState::Done),
            "Done must not be a board column"
        );
    }

    #[test]
    fn into_slice_carries_fields_and_derived_state() {
        let raw = RawSlice {
            number: 42,
            title: "Wire the thing".to_string(),
            assignee: Some("octocat".to_string()),
            assignee_avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            linked_prs: vec![linked_pr()],
            ..ready_raw()
        };

        let slice = raw.into_slice();

        assert_eq!(slice.number, 42);
        assert_eq!(slice.title, "Wire the thing");
        assert_eq!(slice.url, "https://github.com/funkode-io/zfirot/issues/1");
        assert_eq!(
            slice.prd.as_ref().map(|prd| prd.title.as_str()),
            Some("A PRD")
        );
        assert_eq!(slice.assignee.as_deref(), Some("octocat"));
        assert_eq!(
            slice.assignee_avatar_url.as_deref(),
            Some("https://avatars.githubusercontent.com/u/1?v=4")
        );
        assert_eq!(slice.state, SliceState::Wip);
        assert_eq!(slice.linked_prs, vec![linked_pr()]);
    }

    #[test]
    fn into_slice_is_done_for_a_closed_issue() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice().state, SliceState::Done);
    }

    /// A blocker reference helper for the reverse-edge tests.
    fn dep(number: u64) -> DependencyRef {
        DependencyRef {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        }
    }

    fn raw_with(number: u64, blockers: Vec<DependencyRef>) -> RawSlice {
        RawSlice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            blockers,
            ..ready_raw()
        }
    }

    /// `resolve_unblocks` fills each Slice's reverse edge from the board's
    /// blocker edges, in board order, and omits references to issues outside the
    /// fetched set.
    #[test]
    fn resolve_unblocks_derives_the_reverse_edge_across_the_board() {
        // #4 blocks #6; #6 blocks #9; #9 also lists #99 (absent from the board)
        // as a blocker, which must not produce any reverse edge.
        let mut board = vec![
            raw_with(4, vec![]),
            raw_with(6, vec![dep(4)]),
            raw_with(9, vec![dep(6), dep(99)]),
        ];

        resolve_unblocks(&mut board);

        let unblocks = |number: u64| -> Vec<u64> {
            board
                .iter()
                .find(|s| s.number == number)
                .unwrap()
                .unblocks
                .iter()
                .map(|d| d.number)
                .collect()
        };

        // #4 unblocks #6; #6 unblocks #9; #9 unblocks nothing.
        assert_eq!(unblocks(4), vec![6]);
        assert_eq!(unblocks(6), vec![9]);
        assert_eq!(unblocks(9), Vec::<u64>::new());
        // The absent blocker #99 produced no reverse edge.
        assert!(board.iter().all(|s| s.number != 99));
    }

    /// A Slice blocking several others collects all of them, in board order,
    /// each carrying the dependent's number and url for the badge link.
    #[test]
    fn resolve_unblocks_collects_all_dependents_in_order() {
        let mut board = vec![
            raw_with(1, vec![]),
            raw_with(5, vec![dep(1)]),
            raw_with(3, vec![dep(1)]),
        ];

        resolve_unblocks(&mut board);

        let one = board.iter().find(|s| s.number == 1).unwrap();
        assert_eq!(
            one.unblocks,
            vec![dep(5), dep(3)],
            "dependents follow board input order with their refs"
        );
    }

    /// `resolve_unblocks` is pure: re-running it after the blocker edges change
    /// resets each Slice's reverse edge rather than leaving stale data behind.
    #[test]
    fn resolve_unblocks_clears_stale_reverse_edges_on_rerun() {
        // First pass: #6 lists #4 as a blocker, so #4 unblocks #6.
        let mut board = vec![raw_with(4, vec![]), raw_with(6, vec![dep(4)])];
        resolve_unblocks(&mut board);
        let four = board.iter().find(|s| s.number == 4).unwrap();
        assert_eq!(four.unblocks, vec![dep(6)]);

        // The blocker edge is removed; re-running must clear #4's reverse edge.
        board
            .iter_mut()
            .find(|s| s.number == 6)
            .unwrap()
            .blockers
            .clear();
        resolve_unblocks(&mut board);

        let four = board.iter().find(|s| s.number == 4).unwrap();
        assert!(
            four.unblocks.is_empty(),
            "a Slice that no longer blocks anything has no stale reverse edge"
        );
    }

    /// A Slice in a given state, for exercising the summary counts.
    fn slice_in(number: u64, state: SliceState) -> Slice {
        Slice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd: None,
            assignee: None,
            assignee_avatar_url: None,
            state,
            blockers: vec![],
            unblocks: vec![],
            linked_prs: vec![],
        }
    }

    #[test]
    fn board_summary_counts_each_state_and_ignores_done() {
        let slices = vec![
            slice_in(1, SliceState::Ready),
            slice_in(2, SliceState::Ready),
            slice_in(3, SliceState::Wip),
            slice_in(4, SliceState::Blocked),
            slice_in(5, SliceState::Blocked),
            slice_in(6, SliceState::Blocked),
            // Done Slices are hidden from the board, so they count for nothing.
            slice_in(7, SliceState::Done),
            slice_in(8, SliceState::Done),
        ];

        let summary = BoardSummary::from_slices(&slices);

        assert_eq!(summary.ready, 2, "two Ready Slices");
        assert_eq!(summary.wip, 1, "one WIP Slice");
        assert_eq!(summary.blocked, 3, "three Blocked Slices");
        assert_eq!(
            summary.total(),
            6,
            "the total is the visible board only (Done excluded)"
        );
    }

    #[test]
    fn board_summary_of_an_empty_board_is_all_zero() {
        let summary = BoardSummary::from_slices(&[]);

        assert_eq!(summary, BoardSummary::default());
        assert_eq!(summary.total(), 0);
    }
}
