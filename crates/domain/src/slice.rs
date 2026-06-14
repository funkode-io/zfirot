use serde::{Deserialize, Serialize};

/// The derived state of a [`Slice`] on the board.
///
/// Precedence is Blocked > WIP > Ready. `Done` (a closed Slice) is hidden from
/// the board and is therefore not represented here. The state is a pure
/// derivation over current GitHub data (computed in a later slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SliceState {
    /// Blockers all closed, no open linked PR, and no assignee.
    Ready,
    /// An open Pull Request is linked to the Slice.
    Wip,
    /// At least one open "blocked by" dependency.
    Blocked,
}

impl SliceState {
    /// Board column order, left to right.
    pub const ALL: [SliceState; 3] = [SliceState::Ready, SliceState::Wip, SliceState::Blocked];
}

/// A read model of a GitHub issue that is a Slice of a PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// Title of the parent PRD, when known.
    pub prd_title: Option<String>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
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
    /// `true` when the issue is closed; a closed Slice is Done and hidden.
    pub closed: bool,
    /// Title of the parent PRD, when known.
    pub prd_title: Option<String>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// `true` when an open Pull Request is linked via its closing reference.
    pub has_open_linked_pr: bool,
    /// Number of "blocked by" dependencies that are still open.
    pub open_blocker_count: u32,
}

impl RawSlice {
    /// Project this raw issue into a board [`Slice`], or `None` when it is Done
    /// (closed) and therefore hidden from the board.
    pub fn into_slice(self) -> Option<Slice> {
        let state = self.derive_state()?;
        Some(Slice {
            number: self.number,
            title: self.title,
            prd_title: self.prd_title,
            assignee: self.assignee,
            state,
        })
    }

    /// The pure `SliceState` derivation. Returns `None` for Done (closed)
    /// Slices, which are hidden from the board.
    ///
    /// Precedence is Blocked > WIP > Ready:
    /// - **Blocked**: at least one open "blocked by" dependency.
    /// - **WIP**: an open linked PR, or an assignee has claimed it to start work
    ///   (an assigned Slice is by definition no longer Ready).
    /// - **Ready**: all blockers closed, no open linked PR, and no assignee.
    fn derive_state(&self) -> Option<SliceState> {
        if self.closed {
            return None;
        }
        if self.open_blocker_count > 0 {
            return Some(SliceState::Blocked);
        }
        if self.has_open_linked_pr || self.assignee.is_some() {
            return Some(SliceState::Wip);
        }
        Some(SliceState::Ready)
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
            closed: false,
            prd_title: Some("A PRD".to_string()),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        }
    }

    #[test]
    fn derives_each_state_and_hides_done() {
        struct Case {
            name: &'static str,
            closed: bool,
            assignee: Option<&'static str>,
            has_open_linked_pr: bool,
            open_blocker_count: u32,
            expected: Option<SliceState>,
        }

        let cases = [
            Case {
                name: "no blockers, no PR, no assignee -> Ready",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: Some(SliceState::Ready),
            },
            Case {
                name: "open linked PR -> WIP",
                closed: false,
                assignee: None,
                has_open_linked_pr: true,
                open_blocker_count: 0,
                expected: Some(SliceState::Wip),
            },
            Case {
                name: "assigned but no PR -> WIP (no longer Ready)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: Some(SliceState::Wip),
            },
            Case {
                name: "open blocker -> Blocked",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 1,
                expected: Some(SliceState::Blocked),
            },
            Case {
                name: "Blocked outranks WIP (PR + open blocker)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: true,
                open_blocker_count: 2,
                expected: Some(SliceState::Blocked),
            },
            Case {
                name: "WIP outranks Ready (assignee present)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: Some(SliceState::Wip),
            },
            Case {
                name: "closed -> Done, hidden from the board",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                expected: None,
            },
            Case {
                name: "closed wins even with an open blocker",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 3,
                expected: None,
            },
        ];

        for case in cases {
            let raw = RawSlice {
                closed: case.closed,
                assignee: case.assignee.map(str::to_string),
                has_open_linked_pr: case.has_open_linked_pr,
                open_blocker_count: case.open_blocker_count,
                ..ready_raw()
            };

            assert_eq!(raw.derive_state(), case.expected, "{}", case.name);
        }
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

        let slice = raw.into_slice().expect("an open Slice is on the board");

        assert_eq!(slice.number, 42);
        assert_eq!(slice.title, "Wire the thing");
        assert_eq!(slice.prd_title.as_deref(), Some("A PRD"));
        assert_eq!(slice.assignee.as_deref(), Some("octocat"));
        assert_eq!(slice.state, SliceState::Wip);
    }

    #[test]
    fn into_slice_is_none_for_done() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice(), None);
    }
}
