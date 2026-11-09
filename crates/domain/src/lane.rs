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

/// A directed dependency edge between two Slices in the same [`PrdLane`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneGraphEdge {
    /// Upstream blocker issue number.
    pub blocker: u64,
    /// Downstream blocked issue number.
    pub blocked: u64,
}

/// A left-to-right dependency layout of one [`PrdLane`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneGraph {
    /// Ordered columns by dependency depth, left (`0`) to right.
    pub columns: Vec<Vec<Slice>>,
    /// Intra-lane blocker edges only; cross-lane blockers are excluded.
    pub edges: Vec<LaneGraphEdge>,
    /// Slices with no honest position on the dependency axis: not workable, and
    /// with no same-lane edge in either direction. They appear in no column and
    /// no edge, so the leftmost column only ever means "start here".
    pub rail: Vec<Slice>,
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

/// Project one lane into a left-to-right dependency graph.
///
/// Rules:
/// - A Slice with no same-lane blocker sits in the leftmost column.
/// - Any other Slice sits strictly to the right of every same-lane blocker.
/// - `edges` contains only same-lane blocker links (`blocker -> blocked`).
/// - Cross-lane blockers are ignored for layout and edge rendering.
/// - A Blocked Slice with no same-lane edge in either direction sits in `rail`,
///   in input order, rather than in a column: it has no honest depth, and at
///   depth `0` it would read as "start here". Every other Slice lays out as if
///   the rail Slices were not in the lane.
pub fn derive_lane_graph(lane: &PrdLane) -> LaneGraph {
    use std::collections::{BTreeMap, HashMap, HashSet};

    let slice_numbers: HashSet<u64> = lane.slices.iter().map(|slice| slice.number).collect();
    let blockers_by_slice: HashMap<u64, Vec<u64>> = lane
        .slices
        .iter()
        .map(|slice| {
            let blockers = slice
                .blockers
                .iter()
                .filter_map(|blocker| {
                    slice_numbers
                        .contains(&blocker.number)
                        .then_some(blocker.number)
                })
                .collect();
            (slice.number, blockers)
        })
        .collect();

    let mut edges = Vec::new();
    for slice in &lane.slices {
        if let Some(blockers) = blockers_by_slice.get(&slice.number) {
            for blocker in blockers {
                edges.push(LaneGraphEdge {
                    blocker: *blocker,
                    blocked: slice.number,
                });
            }
        }
    }

    fn dependency_depth(
        number: u64,
        blockers_by_slice: &HashMap<u64, Vec<u64>>,
        cache: &mut HashMap<u64, usize>,
        visiting: &mut HashSet<u64>,
    ) -> usize {
        if let Some(depth) = cache.get(&number) {
            return *depth;
        }
        if visiting.contains(&number) {
            // Defensive cycle guard: treat the cycle entry as a root.
            return 0;
        }

        visiting.insert(number);
        let depth = blockers_by_slice
            .get(&number)
            .map(|blockers| {
                blockers
                    .iter()
                    .map(|blocker| {
                        dependency_depth(*blocker, blockers_by_slice, cache, visiting) + 1
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        visiting.remove(&number);
        cache.insert(number, depth);
        depth
    }

    let connected: HashSet<u64> = edges
        .iter()
        .flat_map(|edge| [edge.blocker, edge.blocked])
        .collect();

    let mut cache = HashMap::new();
    let mut columns_by_depth: BTreeMap<usize, Vec<Slice>> = BTreeMap::new();
    let mut rail = Vec::new();
    for slice in &lane.slices {
        if slice.state == SliceState::Blocked && !connected.contains(&slice.number) {
            rail.push(slice.clone());
            continue;
        }
        let depth = dependency_depth(
            slice.number,
            &blockers_by_slice,
            &mut cache,
            &mut HashSet::new(),
        );
        columns_by_depth
            .entry(depth)
            .or_default()
            .push(slice.clone());
    }

    LaneGraph {
        columns: columns_by_depth.into_values().collect(),
        edges,
        rail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockedReason, DependencyRef};

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
            assignee_avatar_url: None,
            state,
            blocked_reasons: vec![],
            blockers: vec![],
            unblocks: vec![],
            linked_prs: vec![],
        }
    }

    fn blocker(number: u64) -> DependencyRef {
        DependencyRef {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        }
    }

    fn ready_slice(number: u64, blockers: Vec<DependencyRef>) -> Slice {
        Slice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd: Some(prd(1)),
            assignee: None,
            assignee_avatar_url: None,
            state: SliceState::Ready,
            blocked_reasons: vec![],
            blockers,
            unblocks: vec![],
            linked_prs: vec![],
        }
    }

    fn blocked_slice(number: u64, reason: BlockedReason, blockers: Vec<DependencyRef>) -> Slice {
        Slice {
            state: SliceState::Blocked,
            blocked_reasons: vec![reason],
            ..ready_slice(number, blockers)
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

    #[test]
    fn derive_lane_graph_layouts_and_edges_are_pure() {
        struct Case {
            name: &'static str,
            slices: Vec<Slice>,
            expected_columns: Vec<Vec<u64>>,
            expected_edges: Vec<(u64, u64)>,
            expected_rail: Vec<u64>,
        }

        let cases = vec![
            Case {
                name: "linear chain",
                slices: vec![
                    ready_slice(1, vec![]),
                    ready_slice(2, vec![blocker(1)]),
                    ready_slice(3, vec![blocker(2)]),
                ],
                expected_columns: vec![vec![1], vec![2], vec![3]],
                expected_edges: vec![(1, 2), (2, 3)],
                expected_rail: vec![],
            },
            Case {
                name: "fan out and diamond",
                slices: vec![
                    ready_slice(10, vec![]),
                    ready_slice(11, vec![blocker(10)]),
                    ready_slice(12, vec![blocker(10)]),
                    ready_slice(13, vec![blocker(11), blocker(12)]),
                ],
                expected_columns: vec![vec![10], vec![11, 12], vec![13]],
                expected_edges: vec![(10, 11), (10, 12), (11, 13), (12, 13)],
                expected_rail: vec![],
            },
            Case {
                name: "isolated node",
                slices: vec![ready_slice(20, vec![])],
                expected_columns: vec![vec![20]],
                expected_edges: vec![],
                expected_rail: vec![],
            },
            Case {
                name: "cross lane blocker excluded",
                slices: vec![
                    ready_slice(30, vec![blocker(999)]),
                    ready_slice(31, vec![blocker(30), blocker(1000)]),
                ],
                expected_columns: vec![vec![30], vec![31]],
                expected_edges: vec![(30, 31)],
                expected_rail: vec![],
            },
            Case {
                name: "empty lane",
                slices: vec![],
                expected_columns: vec![],
                expected_edges: vec![],
                expected_rail: vec![],
            },
            Case {
                name: "edge-less non-workable Slices go to the rail, in input order",
                slices: vec![
                    blocked_slice(40, BlockedReason::Deferred, vec![]),
                    blocked_slice(41, BlockedReason::ExternalBlocker, vec![]),
                    blocked_slice(42, BlockedReason::NeedsTriage, vec![]),
                    blocked_slice(43, BlockedReason::LabelledBlocked, vec![]),
                ],
                expected_columns: vec![],
                expected_edges: vec![],
                expected_rail: vec![40, 41, 42, 43],
            },
            Case {
                name: "Dependency-blocked Slice keeps its column and edge",
                slices: vec![
                    ready_slice(50, vec![]),
                    blocked_slice(51, BlockedReason::Dependency, vec![blocker(50)]),
                ],
                expected_columns: vec![vec![50], vec![51]],
                expected_edges: vec![(50, 51)],
                expected_rail: vec![],
            },
            Case {
                name: "Ready edge-less Slice stays a root beside a railed one",
                slices: vec![
                    blocked_slice(60, BlockedReason::Deferred, vec![]),
                    ready_slice(61, vec![]),
                ],
                expected_columns: vec![vec![61]],
                expected_edges: vec![],
                expected_rail: vec![60],
            },
            Case {
                name: "WIP edge-less Slice stays a root",
                slices: vec![Slice {
                    state: SliceState::Wip,
                    ..ready_slice(65, vec![])
                }],
                expected_columns: vec![vec![65]],
                expected_edges: vec![],
                expected_rail: vec![],
            },
            Case {
                name: "non-workable Slice that only unblocks others stays in the graph",
                slices: vec![
                    blocked_slice(70, BlockedReason::Deferred, vec![]),
                    blocked_slice(71, BlockedReason::Dependency, vec![blocker(70)]),
                ],
                expected_columns: vec![vec![70], vec![71]],
                expected_edges: vec![(70, 71)],
                expected_rail: vec![],
            },
            Case {
                name: "workable Slice that only unblocks others stays in the graph",
                slices: vec![
                    ready_slice(75, vec![]),
                    blocked_slice(76, BlockedReason::Dependency, vec![blocker(75)]),
                ],
                expected_columns: vec![vec![75], vec![76]],
                expected_edges: vec![(75, 76)],
                expected_rail: vec![],
            },
            Case {
                name: "Slice blocked only across lanes has no same-lane edge, so it rails",
                slices: vec![
                    ready_slice(80, vec![]),
                    blocked_slice(81, BlockedReason::Dependency, vec![blocker(999)]),
                ],
                expected_columns: vec![vec![80]],
                expected_edges: vec![],
                expected_rail: vec![81],
            },
            Case {
                name: "a chain lays out exactly as without the railed Slice",
                slices: vec![
                    ready_slice(90, vec![]),
                    blocked_slice(91, BlockedReason::Deferred, vec![]),
                    blocked_slice(92, BlockedReason::Dependency, vec![blocker(90)]),
                    blocked_slice(93, BlockedReason::Dependency, vec![blocker(92)]),
                ],
                expected_columns: vec![vec![90], vec![92], vec![93]],
                expected_edges: vec![(90, 92), (92, 93)],
                expected_rail: vec![91],
            },
        ];

        for case in cases {
            let lane = PrdLane {
                prd: Some(prd(1)),
                slices: case.slices,
            };
            let graph = derive_lane_graph(&lane);

            // Whatever the arrangement, Graph view shows exactly the Slices of
            // the lane — the same ones Columns view shows — each once, unchanged.
            let mut laid_out: Vec<Slice> = graph
                .columns
                .iter()
                .flatten()
                .chain(&graph.rail)
                .cloned()
                .collect();
            laid_out.sort_by_key(|slice| slice.number);
            let mut lane_slices = lane.slices.clone();
            lane_slices.sort_by_key(|slice| slice.number);
            assert_eq!(laid_out, lane_slices, "{}", case.name);

            let columns: Vec<Vec<u64>> = graph
                .columns
                .iter()
                .map(|column| column.iter().map(|slice| slice.number).collect())
                .collect();
            let edges: Vec<(u64, u64)> = graph
                .edges
                .iter()
                .map(|edge| (edge.blocker, edge.blocked))
                .collect();

            assert_eq!(columns, case.expected_columns, "{}", case.name);
            assert_eq!(edges, case.expected_edges, "{}", case.name);

            let rail: Vec<u64> = graph.rail.iter().map(|slice| slice.number).collect();
            assert_eq!(rail, case.expected_rail, "{}", case.name);
            assert!(
                graph
                    .edges
                    .iter()
                    .all(|edge| !rail.contains(&edge.blocker) && !rail.contains(&edge.blocked)),
                "{}: a rail Slice drew an edge",
                case.name
            );
        }
    }
}
