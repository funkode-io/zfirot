//! At-a-glance counts of the active board states.

use crate::{Slice, SliceState};

/// How many Slices sit in each active board column. `Done` is hidden from the
/// board, so it is intentionally not counted. A pure derivation over the loaded
/// Slices, so it stays testable and offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoardSummary {
    pub ready: usize,
    pub wip: usize,
    pub blocked: usize,
}

impl BoardSummary {
    /// Count the Slices by their derived state, ignoring `Done`.
    pub fn from_slices(slices: &[Slice]) -> Self {
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

    /// Total of the active (board-visible) Slices.
    pub fn total(&self) -> usize {
        self.ready + self.wip + self.blocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Slice;

    fn slice(number: u64, state: SliceState) -> Slice {
        Slice {
            number,
            title: format!("Slice #{number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd_title: None,
            assignee: None,
            blockers: Vec::new(),
            state,
        }
    }

    #[test]
    fn counts_each_active_state_and_ignores_done() {
        struct Case {
            name: &'static str,
            states: &'static [SliceState],
            ready: usize,
            wip: usize,
            blocked: usize,
        }

        let cases = [
            Case {
                name: "empty board is all zeros",
                states: &[],
                ready: 0,
                wip: 0,
                blocked: 0,
            },
            Case {
                name: "one of each active state",
                states: &[SliceState::Ready, SliceState::Wip, SliceState::Blocked],
                ready: 1,
                wip: 1,
                blocked: 1,
            },
            Case {
                name: "Done is not counted in any column",
                states: &[
                    SliceState::Ready,
                    SliceState::Ready,
                    SliceState::Done,
                    SliceState::Blocked,
                    SliceState::Done,
                ],
                ready: 2,
                wip: 0,
                blocked: 1,
            },
        ];

        for case in cases {
            let slices: Vec<Slice> = case
                .states
                .iter()
                .enumerate()
                .map(|(i, &state)| slice(i as u64, state))
                .collect();

            let summary = BoardSummary::from_slices(&slices);

            assert_eq!(summary.ready, case.ready, "ready: {}", case.name);
            assert_eq!(summary.wip, case.wip, "wip: {}", case.name);
            assert_eq!(summary.blocked, case.blocked, "blocked: {}", case.name);
            assert_eq!(
                summary.total(),
                case.ready + case.wip + case.blocked,
                "total: {}",
                case.name
            );
        }
    }
}
