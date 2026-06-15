use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A reference to another issue on the board, carrying just enough to render a
/// clickable dependency badge: its number (for the label) and url (to open it on
/// GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRef {
    /// The referenced GitHub issue number.
    pub number: u64,
    /// The referenced issue's URL on GitHub, for opening it in a browser.
    pub url: String,
}

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
    /// An open Pull Request is linked to the Slice.
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

/// A read model of a GitHub issue that is a Slice of a PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// Title of the parent PRD, when known.
    pub prd_title: Option<String>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    pub state: SliceState,
    /// The open issues this Slice is blocked by, resolved against the board.
    pub blockers: Vec<IssueRef>,
    /// The reverse edge: the issues that list this Slice as a blocker (the ones
    /// this Slice unblocks), derived across the whole board.
    pub unblocks: Vec<IssueRef>,
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
    /// Title of the parent PRD, when known.
    pub prd_title: Option<String>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// `true` when an open Pull Request is linked via its closing reference.
    pub has_open_linked_pr: bool,
    /// The "blocked by" dependencies that are still open, resolved to their
    /// issue references (number + url) against the board fetched in the same
    /// load. Closed or absent references are omitted upstream.
    pub blockers: Vec<IssueRef>,
}

/// Project a whole board of raw issues into [`Slice`]s, deriving the reverse
/// "unblocks" edges across the board.
///
/// Each Slice carries the open issues it is blocked by (`blockers`) and the
/// issues that list it as a blocker (`unblocks`). The reverse edge is a pure
/// derivation over the resolved `blockers`: for every Slice Y blocked by X, X
/// gains Y in its `unblocks`. Edges to issues outside the resolved set are
/// already omitted by the upstream resolution, so they never appear here.
pub fn derive_board(raws: Vec<RawSlice>) -> Vec<Slice> {
    let mut unblocks: HashMap<u64, Vec<IssueRef>> = HashMap::new();
    for raw in &raws {
        let blocked = IssueRef {
            number: raw.number,
            url: raw.url.clone(),
        };
        for blocker in &raw.blockers {
            unblocks
                .entry(blocker.number)
                .or_default()
                .push(blocked.clone());
        }
    }

    raws.into_iter()
        .map(|raw| {
            let number = raw.number;
            let mut slice = raw.into_slice();
            slice.unblocks = unblocks.remove(&number).unwrap_or_default();
            slice
        })
        .collect()
}

impl RawSlice {
    /// Project this raw issue into a [`Slice`] with its derived [`SliceState`].
    ///
    /// The reverse `unblocks` edge is left empty here because it can only be
    /// derived across the whole board; [`derive_board`] fills it in.
    pub fn into_slice(self) -> Slice {
        let state = self.derive_state();
        Slice {
            number: self.number,
            title: self.title,
            url: self.url,
            prd_title: self.prd_title,
            assignee: self.assignee,
            state,
            blockers: self.blockers,
            unblocks: Vec::new(),
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
        if self.has_open_linked_pr || self.assignee.is_some() {
            return SliceState::Wip;
        }
        SliceState::Ready
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
            prd_title: Some("A PRD".to_string()),
            assignee: None,
            has_open_linked_pr: false,
            blockers: Vec::new(),
        }
    }

    /// Build `count` placeholder blocker references for state-derivation cases
    /// (only their presence matters to the derivation).
    fn blockers(count: u32) -> Vec<IssueRef> {
        (0..count)
            .map(|i| IssueRef {
                number: 100 + u64::from(i),
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
            open_blocker_count: u32,
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
                has_open_linked_pr: case.has_open_linked_pr,
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
            has_open_linked_pr: true,
            ..ready_raw()
        };

        let slice = raw.into_slice();

        assert_eq!(slice.number, 42);
        assert_eq!(slice.title, "Wire the thing");
        assert_eq!(slice.url, "https://github.com/funkode-io/zfirot/issues/1");
        assert_eq!(slice.prd_title.as_deref(), Some("A PRD"));
        assert_eq!(slice.assignee.as_deref(), Some("octocat"));
        assert_eq!(slice.state, SliceState::Wip);
        assert!(
            slice.unblocks.is_empty(),
            "per-slice projection cannot know the reverse edge"
        );
    }

    #[test]
    fn into_slice_is_done_for_a_closed_issue() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice().state, SliceState::Done);
    }

    /// A raw Slice numbered `number` blocked by `blocked_by`, for board tests.
    fn raw_blocked_by(number: u64, blocked_by: &[u64]) -> RawSlice {
        RawSlice {
            number,
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            blockers: blocked_by
                .iter()
                .map(|&n| IssueRef {
                    number: n,
                    url: format!("https://github.com/funkode-io/zfirot/issues/{n}"),
                })
                .collect(),
            ..ready_raw()
        }
    }

    fn slice_number<'a>(slices: &'a [Slice], number: u64) -> &'a Slice {
        slices
            .iter()
            .find(|s| s.number == number)
            .unwrap_or_else(|| panic!("no slice #{number} in the board"))
    }

    #[test]
    fn resolve_board_derives_reverse_unblocks_edges() {
        // #6 and #7 are both blocked by #4; #7 is also blocked by #6. The board
        // is intentionally out of order to show the derivation is order-free.
        let raws = vec![
            raw_blocked_by(7, &[4, 6]),
            raw_blocked_by(4, &[]),
            raw_blocked_by(6, &[4]),
        ];

        let board = derive_board(raws);

        // #4 unblocks both #6 and #7 (in board order), and is itself Ready.
        let four = slice_number(&board, 4);
        assert_eq!(four.state, SliceState::Ready);
        assert!(four.blockers.is_empty());
        assert_eq!(
            four.unblocks.iter().map(|r| r.number).collect::<Vec<_>>(),
            vec![7, 6]
        );
        assert_eq!(
            four.unblocks[0].url,
            "https://github.com/funkode-io/zfirot/issues/7",
            "the reverse edge carries the blocked issue's url for a clickable badge"
        );

        // #6 is blocked by #4 and unblocks #7.
        let six = slice_number(&board, 6);
        assert_eq!(six.state, SliceState::Blocked);
        assert_eq!(
            six.blockers.iter().map(|r| r.number).collect::<Vec<_>>(),
            vec![4]
        );
        assert_eq!(
            six.unblocks.iter().map(|r| r.number).collect::<Vec<_>>(),
            vec![7]
        );

        // #7 is blocked by #4 and #6, and unblocks nothing.
        let seven = slice_number(&board, 7);
        assert_eq!(
            seven.blockers.iter().map(|r| r.number).collect::<Vec<_>>(),
            vec![4, 6]
        );
        assert!(seven.unblocks.is_empty());
    }

    #[test]
    fn resolve_board_omits_edges_to_absent_blockers() {
        // #9 lists #99 as a blocker, but #99 is not on the board (closed or
        // absent). Such a reference must already be omitted from `blockers`, so
        // no slice ever claims to unblock #99.
        let raws = vec![raw_blocked_by(9, &[])];

        let board = derive_board(raws);

        let nine = slice_number(&board, 9);
        assert!(nine.blockers.is_empty(), "absent references are omitted");
        assert!(
            board.iter().all(|s| s.unblocks.is_empty()),
            "no reverse edge points at an off-board issue"
        );
    }
}
