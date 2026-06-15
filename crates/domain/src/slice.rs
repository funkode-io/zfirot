use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A reference to the parent PRD of a [`Slice`].
///
/// Carries the PRD's identity — issue number, title, and URL — not just its
/// title, so the board can group Slices into stable lanes and link each lane
/// header back to the PRD issue on GitHub. Derived from the native sub-issue
/// parent or the resolved prose `## Parent` reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrdRef {
    /// The PRD's GitHub issue number; the stable key the board groups lanes by.
    pub number: u64,
    pub title: String,
    /// The PRD issue's URL on GitHub, for the lane header link.
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
    /// The parent PRD, when known, carrying its identity for lane grouping.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    pub state: SliceState,
}

/// A board swimlane: the [`Slice`]s that belong to one PRD, or the trailing lane
/// of Slices with no PRD.
///
/// Lanes are produced by [`group_into_lanes`] in the order their PRD is first
/// seen, so the board is stable across loads; the no-PRD lane is always last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrdLane {
    /// The PRD this lane groups, or `None` for the "No PRD" lane.
    pub prd: Option<PrdRef>,
    /// The lane's Slices, in their original order and excluding Done.
    pub slices: Vec<Slice>,
}

/// Group Slices into one lane per PRD, plus a trailing lane for Slices with no
/// PRD, for the board's swimlane layout.
///
/// Lanes appear in the order each PRD is first encountered (keyed by the PRD
/// issue number, so a prose and a native parent for the same PRD share a lane);
/// the no-PRD lane, when present, is always last. Done (closed) Slices are
/// excluded because the board never shows them, and a lane that would be empty
/// after that exclusion is dropped.
pub fn group_into_lanes(slices: impl IntoIterator<Item = Slice>) -> Vec<PrdLane> {
    let mut lanes: Vec<PrdLane> = Vec::new();
    let mut lane_by_prd: HashMap<u64, usize> = HashMap::new();
    let mut no_prd: Vec<Slice> = Vec::new();

    for slice in slices {
        if slice.state == SliceState::Done {
            continue;
        }
        match slice.prd.clone() {
            Some(prd) => {
                let index = *lane_by_prd.entry(prd.number).or_insert_with(|| {
                    lanes.push(PrdLane {
                        prd: Some(prd),
                        slices: Vec::new(),
                    });
                    lanes.len() - 1
                });
                lanes[index].slices.push(slice);
            }
            None => no_prd.push(slice),
        }
    }

    if !no_prd.is_empty() {
        lanes.push(PrdLane {
            prd: None,
            slices: no_prd,
        });
    }

    lanes
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
    /// The parent PRD, when known, carrying its identity for lane grouping.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// `true` when an open Pull Request is linked via its closing reference.
    pub has_open_linked_pr: bool,
    /// Number of "blocked by" dependencies that are still open.
    pub open_blocker_count: u32,
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
        if self.open_blocker_count > 0 {
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
            prd: Some(a_prd()),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        }
    }

    /// A PRD reference used by the baseline raw Slice.
    fn a_prd() -> PrdRef {
        PrdRef {
            number: 100,
            title: "A PRD".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/100".to_string(),
        }
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
                open_blocker_count: case.open_blocker_count,
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
        assert_eq!(slice.prd, Some(a_prd()));
        assert_eq!(slice.assignee.as_deref(), Some("octocat"));
        assert_eq!(slice.state, SliceState::Wip);
    }

    #[test]
    fn into_slice_is_done_for_a_closed_issue() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice().state, SliceState::Done);
    }

    /// A Ready Slice tagged with the given PRD (or none), for lane grouping.
    fn slice_for(number: u64, prd: Option<PrdRef>) -> Slice {
        Slice {
            number,
            title: format!("Slice #{number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd,
            assignee: None,
            state: SliceState::Ready,
        }
    }

    fn prd(number: u64) -> PrdRef {
        PrdRef {
            number,
            title: format!("PRD #{number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        }
    }

    #[test]
    fn groups_slices_into_one_lane_per_prd_in_first_seen_order() {
        let slices = vec![
            slice_for(1, Some(prd(10))),
            slice_for(2, Some(prd(20))),
            slice_for(3, Some(prd(10))),
        ];

        let lanes = group_into_lanes(slices);

        assert_eq!(lanes.len(), 2, "one lane per distinct PRD");
        assert_eq!(
            lanes[0].prd,
            Some(prd(10)),
            "PRD 10 first, it was seen first"
        );
        assert_eq!(
            lanes[0].slices.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 3],
            "both PRD-10 Slices share its lane, in order"
        );
        assert_eq!(lanes[1].prd, Some(prd(20)));
        assert_eq!(
            lanes[1].slices.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn slices_with_no_prd_go_to_a_trailing_lane() {
        let slices = vec![
            slice_for(1, None),
            slice_for(2, Some(prd(10))),
            slice_for(3, None),
        ];

        let lanes = group_into_lanes(slices);

        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0].prd, Some(prd(10)), "PRD lanes come before No PRD");
        assert_eq!(lanes[1].prd, None, "the No PRD lane is always last");
        assert_eq!(
            lanes[1].slices.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn done_slices_are_excluded_and_empty_lanes_dropped() {
        let done = Slice {
            state: SliceState::Done,
            ..slice_for(1, Some(prd(10)))
        };
        let slices = vec![done, slice_for(2, Some(prd(20)))];

        let lanes = group_into_lanes(slices);

        assert_eq!(
            lanes.len(),
            1,
            "the all-Done PRD-10 lane is dropped, only PRD 20 remains"
        );
        assert_eq!(lanes[0].prd, Some(prd(20)));
    }

    #[test]
    fn no_slices_yields_no_lanes() {
        assert!(group_into_lanes(Vec::new()).is_empty());
    }
}
