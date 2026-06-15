use serde::{Deserialize, Serialize};

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

/// A Slice that blocks another one: its issue number and, when known, its title.
///
/// Carried on a [`Slice`] so a Blocked card can list exactly which Slices are
/// holding it up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocker {
    /// The GitHub issue number of the blocking Slice.
    pub number: u64,
    /// The blocking Slice's title, when it could be resolved.
    pub title: Option<String>,
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
    /// The still-open Slices blocking this one. Empty unless [`SliceState::Blocked`].
    pub blockers: Vec<Blocker>,
    pub state: SliceState,
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
    /// The still-open "blocked by" dependencies, by issue number (and title when
    /// resolved). A non-empty list makes the Slice [`SliceState::Blocked`].
    pub blockers: Vec<Blocker>,
}

impl RawSlice {
    /// Project this raw issue into a [`Slice`] with its derived [`SliceState`].
    pub fn into_slice(self) -> Slice {
        let state = self.derive_state();
        Slice {
            number: self.number,
            title: self.title,
            url: self.url,
            prd_title: self.prd_title,
            assignee: self.assignee,
            blockers: self.blockers,
            state,
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

    /// `n` placeholder open blockers, for the state-precedence table.
    fn n_blockers(n: u32) -> Vec<Blocker> {
        (0..n)
            .map(|i| Blocker {
                number: 100 + u64::from(i),
                title: None,
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
                blockers: n_blockers(case.open_blocker_count),
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
        assert!(slice.blockers.is_empty());
        assert_eq!(slice.state, SliceState::Wip);
    }

    #[test]
    fn into_slice_carries_blockers_for_a_blocked_slice() {
        let raw = RawSlice {
            blockers: vec![Blocker {
                number: 7,
                title: Some("Set up auth".to_string()),
            }],
            ..ready_raw()
        };

        let slice = raw.into_slice();

        assert_eq!(slice.state, SliceState::Blocked);
        assert_eq!(slice.blockers.len(), 1);
        assert_eq!(slice.blockers[0].number, 7);
        assert_eq!(slice.blockers[0].title.as_deref(), Some("Set up auth"));
    }

    #[test]
    fn into_slice_is_done_for_a_closed_issue() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice().state, SliceState::Done);
    }
}
