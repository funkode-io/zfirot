use dioxus::prelude::*;
use domain::{
    derive_lane_graph, LaneGraphEdge, PrdLane as DomainPrdLane, PrdRef, Slice, SliceState,
};

use super::{state_badge_class, state_label, BoardColumn, SliceCard};

/// One swimlane: a collapsible PRD header above the Ready / WIP / Blocked
/// columns holding that PRD's Slices. A lane with no PRD (`prd` is `None`)
/// renders a plain "No PRD" header. Built on the daisyUI `collapse` component
/// (a checkbox toggle + `collapse-arrow` chevron), matching the "other open
/// issues" accordion — so the PRD link is a sibling of the toggle, never nested
/// inside a `<button>`. The lane starts expanded and the whole header toggles it.
/// Collapsing the lane hides the columns and summarises them as coloured
/// per-state count badges (shown only while collapsed, via `group-has-[:checked]:hidden`);
/// the state name is shown on hover via each badge's `title`.
///
/// `highlighted` / `on_highlight` carry the board-wide "highlighted issue" so a
/// dependency badge can highlight its referenced card even in another lane.
#[component]
pub fn PrdLane(
    prd: Option<PrdRef>,
    slices: Vec<Slice>,
    graph_view: bool,
    on_assign: EventHandler<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    let total_slices = slices.len();
    let graph = graph_view.then(|| {
        derive_lane_graph(&DomainPrdLane {
            prd: prd.clone(),
            slices: slices.clone(),
        })
    });

    // Bucket each Slice into its board column exactly once, so a Slice is cloned
    // at most once per render regardless of how many columns there are.
    let mut buckets: Vec<(SliceState, Vec<Slice>)> = SliceState::BOARD
        .iter()
        .map(|&state| (state, Vec::new()))
        .collect();
    for slice in &slices {
        if let Some((_, bucket)) = buckets.iter_mut().find(|(state, _)| *state == slice.state) {
            bucket.push(slice.clone());
        }
    }

    // Per-state counts for the collapsed summary, in board column order.
    let counts: Vec<(SliceState, usize)> = buckets
        .iter()
        .map(|(state, bucket)| (*state, bucket.len()))
        .collect();

    rsx! {
        section { class: "collapse collapse-arrow bg-base-200 rounded-box group",
            // Checkbox toggle drives the daisyUI collapse. Starts `checked` so
            // the lane opens expanded. `aria-label` names the control for
            // assistive tech. The `group` on the section lets the collapsed-only
            // summary badges hide themselves via `group-has-[:checked]:hidden`
            // when this checkbox is checked (expanded).
            input { r#type: "checkbox", checked: true, "aria-label": "Toggle lane" }
            div { class: "collapse-title flex min-w-0 items-center gap-3",
                match prd {
                    Some(prd) => rsx! {
                        // `relative z-10` lifts the link above the collapse
                        // checkbox so clicking it navigates; `stop_propagation`
                        // keeps that click from also toggling the lane.
                        a {
                            class: "link link-hover font-semibold truncate relative z-10",
                            href: "{prd.url}",
                            onclick: move |e: Event<MouseData>| e.stop_propagation(),
                            "#{prd.number} {prd.title}"
                        }
                        span { class: "badge badge-sm badge-neutral shrink-0", "{slices_pill_label(total_slices)}" }
                    },
                    None => rsx! {
                        span { class: "font-semibold opacity-70", "No PRD" }
                    },
                }
                div { class: "flex items-center gap-1 group-has-[:checked]:hidden",
                    for (state , count) in counts.iter().copied() {
                        span {
                            class: "badge badge-sm {state_badge_class(state)}",
                            title: "{state_label(state)}",
                            "{count}"
                        }
                    }
                }
            }
            div { class: "collapse-content",
                if let Some(graph) = graph {
                    GraphLane {
                        columns: graph.columns,
                        edges: graph.edges,
                        on_assign,
                        highlighted,
                        on_highlight,
                    }
                } else {
                    div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                        for (state , bucket) in buckets {
                            BoardColumn {
                                state,
                                label: state_label(state).to_string(),
                                slices: bucket,
                                on_assign,
                                highlighted,
                                on_highlight,
                            }
                        }
                    }
                }
            }
        }
    }
}

fn slices_pill_label(total_slices: usize) -> String {
    if total_slices == 1 {
        "1 slice".to_string()
    } else {
        format!("{total_slices} slices")
    }
}

#[component]
fn GraphLane(
    columns: Vec<Vec<Slice>>,
    edges: Vec<LaneGraphEdge>,
    on_assign: EventHandler<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    use std::collections::HashMap;

    const NODE_WIDTH: usize = 320;
    const NODE_HEIGHT: usize = 96;
    const COLUMN_GAP: usize = 40;
    const ROW_GAP: usize = 24;
    const PADDING: usize = 16;

    let max_rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    let canvas_width =
        columns.len() * NODE_WIDTH + columns.len().saturating_sub(1) * COLUMN_GAP + PADDING * 2;
    let canvas_height = max_rows * NODE_HEIGHT + max_rows.saturating_sub(1) * ROW_GAP + PADDING * 2;

    let mut positions: HashMap<u64, (usize, usize)> = HashMap::new();
    for (col_idx, column) in columns.iter().enumerate() {
        for (row_idx, slice) in column.iter().enumerate() {
            positions.insert(slice.number, (col_idx, row_idx));
        }
    }

    rsx! {
        div { class: "overflow-x-auto",
            if columns.is_empty() {
                div { class: "text-sm opacity-60", "No active Slices" }
            } else {
                div {
                    class: "relative",
                    style: "width: {canvas_width}px; height: {canvas_height}px;",
                    svg {
                        class: "absolute inset-0 pointer-events-none",
                        width: "{canvas_width}",
                        height: "{canvas_height}",
                        view_box: "0 0 {canvas_width} {canvas_height}",
                        defs {
                            marker {
                                id: "lane-arrow",
                                marker_width: "10",
                                marker_height: "7",
                                ref_x: "10",
                                ref_y: "3.5",
                                orient: "auto",
                                polygon {
                                    class: "fill-base-content/50",
                                    points: "0 0, 10 3.5, 0 7",
                                }
                            }
                        }
                        for edge in edges {
                            if let (Some(&(from_col, from_row)), Some(&(to_col, to_row))) =
                                (positions.get(&edge.blocker), positions.get(&edge.blocked))
                            {
                                {
                                    let from_x =
                                        PADDING + (from_col * (NODE_WIDTH + COLUMN_GAP)) + NODE_WIDTH;
                                    let from_y = PADDING
                                        + (from_row * (NODE_HEIGHT + ROW_GAP))
                                        + (NODE_HEIGHT / 2);
                                    let to_x = PADDING + (to_col * (NODE_WIDTH + COLUMN_GAP));
                                    let to_y = PADDING
                                        + (to_row * (NODE_HEIGHT + ROW_GAP))
                                        + (NODE_HEIGHT / 2);
                                    let mid_x = from_x + ((to_x - from_x) / 2);
                                    rsx! {
                                        path {
                                            d: "M {from_x} {from_y} C {mid_x} {from_y}, {mid_x} {to_y}, {to_x} {to_y}",
                                            class: "stroke-base-content/40",
                                            "stroke-width": "2",
                                            fill: "none",
                                            marker_end: "url(#lane-arrow)",
                                        }
                                    }
                                }
                            }
                        }
                    }
                    for (col_idx , column) in columns.iter().enumerate() {
                        for (row_idx , slice) in column.iter().enumerate() {
                            div {
                                key: "{slice.number}",
                                class: "absolute",
                                style: "left: {PADDING + (col_idx * (NODE_WIDTH + COLUMN_GAP))}px; top: {PADDING + (row_idx * (NODE_HEIGHT + ROW_GAP))}px; width: {NODE_WIDTH}px;",
                                SliceCard {
                                    slice: slice.clone(),
                                    on_assign,
                                    highlighted,
                                    on_highlight,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
