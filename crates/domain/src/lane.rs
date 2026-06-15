use crate::{PrdRef, Slice, SliceState};

/// One swimlane of the board: a PRD (or the "No PRD" group) and the Slices that
/// belong to it. The state columns (Ready/WIP/Blocked) are rendered *within* a
/// lane by the presentation layer; this read model only carries the grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrdLane {
    /// The PRD owning this lane, or `None` for the trailing "No PRD" lane.
    pub prd: Option<PrdRef>,
    /// The Slices in this lane, in input order.
    pub slices: Vec<Slice>,
}

/// Group Slices into one lane per PRD, in first-seen order, with a trailing
/// "No PRD" lane for Slices with no parent PRD.
///
/// A pure derivation:
/// - `Done` Slices are excluded (they are hidden from the active board).
/// - PRD lanes appear in the order their first Slice is seen; the "No PRD" lane,
///   when present, always comes last.
/// - Lanes left empty after excluding `Done` are dropped, so an all-`Done` PRD
///   produces no lane.
pub fn group_into_lanes(slices: impl IntoIterator<Item = Slice>) -> Vec<PrdLane> {
    // PRD lanes in first-seen order, keyed by PRD number; the no-PRD group is
    // accumulated separately so it can be appended last.
    let mut order: Vec<u64> = Vec::new();
    let mut prd_by_number: std::collections::HashMap<u64, PrdRef> =
        std::collections::HashMap::new();
    let mut slices_by_number: std::collections::HashMap<u64, Vec<Slice>> =
        std::collections::HashMap::new();
    let mut no_prd: Vec<Slice> = Vec::new();

    for slice in slices {
        if slice.state == SliceState::Done {
            continue;
        }
        match slice.prd.clone() {
            Some(prd) => {
                if !slices_by_number.contains_key(&prd.number) {
                    order.push(prd.number);
                    prd_by_number.insert(prd.number, prd.clone());
                }
                slices_by_number.entry(prd.number).or_default().push(slice);
            }
            None => no_prd.push(slice),
        }
    }

    let mut lanes: Vec<PrdLane> = order
        .into_iter()
        .map(|number| PrdLane {
            prd: prd_by_number.remove(&number),
            slices: slices_by_number.remove(&number).unwrap_or_default(),
        })
        .collect();

    if !no_prd.is_empty() {
        lanes.push(PrdLane {
            prd: None,
            slices: no_prd,
        });
    }

    lanes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prd(number: u64) -> PrdRef {
        PrdRef {
            number,
            title: format!("PRD {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        }
    }

    fn slice(number: u64, prd: Option<PrdRef>, state: SliceState) -> Slice {
        Slice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd,
            assignee: None,
            state,
        }
    }

    /// Lanes come out in first-seen PRD order, the "No PRD" lane is last, and
    /// `Done` Slices are excluded (dropping any lane left empty).
    #[test]
    fn groups_in_first_seen_order_with_trailing_no_prd_lane() {
        let input = vec![
            slice(4, Some(prd(1)), SliceState::Ready),
            slice(5, Some(prd(10)), SliceState::Wip),
            slice(11, None, SliceState::Ready),
            slice(3, Some(prd(1)), SliceState::Blocked),
            slice(6, Some(prd(10)), SliceState::Ready),
            // Excluded: Done, and the only Slice of PRD 99 -> no lane for 99.
            slice(7, Some(prd(99)), SliceState::Done),
        ];

        let lanes = group_into_lanes(input);

        let shape: Vec<(Option<u64>, Vec<u64>)> = lanes
            .iter()
            .map(|lane| {
                (
                    lane.prd.as_ref().map(|p| p.number),
                    lane.slices.iter().map(|s| s.number).collect(),
                )
            })
            .collect();

        assert_eq!(
            shape,
            vec![
                (Some(1), vec![4, 3]),
                (Some(10), vec![5, 6]),
                (None, vec![11]),
            ]
        );
    }

    #[test]
    fn empty_input_yields_no_lanes() {
        assert!(group_into_lanes(Vec::<Slice>::new()).is_empty());
    }

    #[test]
    fn an_all_done_prd_produces_no_lane() {
        let input = vec![
            slice(1, Some(prd(1)), SliceState::Done),
            slice(2, Some(prd(1)), SliceState::Done),
        ];
        assert!(group_into_lanes(input).is_empty());
    }
}
