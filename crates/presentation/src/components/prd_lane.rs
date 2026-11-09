use dioxus::prelude::*;
use domain::{
    derive_lane_graph, order_blocked_column, LaneGraphEdge, PrdLane as DomainPrdLane, PrdRef,
    Slice, SliceState,
};

use super::{state_badge_class, state_label, BoardColumn, SliceCard};

/// One swimlane: a collapsible PRD header above the Ready / WIP / Blocked
/// columns holding that PRD's Slices. A lane with no PRD (`prd` is `None`)
/// renders a plain "No PRD" header, and a **Suggested PRD**'s header is dimmed and
/// badged "Unconfirmed", with guidance and a link for settling it on GitHub, so a
/// guess is never mistaken for a fact. Built on the
/// daisyUI `collapse` component (a checkbox toggle + `collapse-arrow` chevron), matching the "other open
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
    let suggested = prd.as_ref().is_some_and(|prd| prd.suggested);
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

    // The Blocked column reads top-down as "soonest to free up": dependencies
    // first (they clear themselves), decisions last.
    if let Some((_, blocked)) = buckets
        .iter_mut()
        .find(|(state, _)| *state == SliceState::Blocked)
    {
        order_blocked_column(blocked);
    }

    // Per-state counts for the collapsed summary, in board column order.
    let counts: Vec<(SliceState, usize)> = buckets
        .iter()
        .map(|(state, bucket)| (*state, bucket.len()))
        .collect();

    rsx! {
        section {
            class: "collapse collapse-arrow bg-base-200 rounded-box group",
            class: if suggested { "border-l-4 border-warning" },
            // Checkbox toggle drives the daisyUI collapse. Starts `checked` so
            // the lane opens expanded. `aria-label` names the control for
            // assistive tech. The `group` on the section lets the collapsed-only
            // summary badges hide themselves via `group-has-[:checked]:hidden`
            // when this checkbox is checked (expanded).
            input { r#type: "checkbox", checked: true, "aria-label": "Toggle lane" }
            div {
                class: "collapse-title flex min-w-0 items-center gap-3",
                class: if suggested { "flex-wrap" },
                match prd {
                    Some(prd) => rsx! {
                        // `relative z-10` lifts the link above the collapse
                        // checkbox so clicking it navigates; `stop_propagation`
                        // keeps that click from also toggling the lane.
                        a {
                            class: "link link-hover font-semibold truncate relative z-10",
                            class: if prd.suggested { "italic opacity-70" },
                            href: "{prd.url}",
                            onclick: move |e: Event<MouseData>| e.stop_propagation(),
                            "#{prd.number} {prd.title}"
                        }
                        if prd.suggested {
                            span {
                                class: "badge badge-sm badge-warning badge-outline shrink-0",
                                title: "Reads like a PRD by its headings, but has no \"prd\" label",
                                "Unconfirmed"
                            }
                        }
                        span { class: "badge badge-sm badge-neutral shrink-0", "{slices_pill_label(total_slices)}" }
                        if let Some(guidance) = guidance_for(&prd) {
                            p { class: "basis-full text-xs font-normal",
                                "{guidance} "
                                a {
                                    class: "link relative z-10",
                                    href: "{prd.url}",
                                    onclick: move |e: Event<MouseData>| e.stop_propagation(),
                                    "Set the label on GitHub"
                                }
                            }
                        }
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
                        rail: graph.rail,
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

/// What a PRD's Lane header tells the user about settling it, if anything: only a
/// Suggested PRD has a guess to settle. The board only reads these labels, so that
/// is theirs to do on GitHub.
fn guidance_for(prd: &PrdRef) -> Option<&'static str> {
    prd.suggested.then_some(
        "Guessed from its headings. If this is a PRD, add the \"prd\" label on GitHub. If it \
         is not, add \"not-a-prd\": it stops being suggested, and its Slices stay visible.",
    )
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
    rail: Vec<Slice>,
    on_assign: EventHandler<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    use std::collections::HashMap;

    const NODE_WIDTH: usize = 320;
    const COLUMN_GAP: usize = 56;
    const ROW_GAP: usize = 20;
    // Wider than a column gap, so the rail never reads as the next depth.
    const RAIL_GAP: usize = 96;

    // Real, measured geometry of each rendered node, in viewport pixels:
    // number -> (x, y, width, height). Edges are drawn from these anchors so an
    // arrow tracks the card's actual height instead of a guessed constant — a
    // straight same-row edge then lands exactly on both card centres and stays
    // visible in the gap, and taller cards never overlap.
    let mut node_rects = use_signal(HashMap::<u64, (f64, f64, f64, f64)>::new);
    // The graph container's own origin (viewport px), so node rects can be
    // re-expressed relative to the SVG overlay that fills it.
    let mut origin = use_signal(|| None::<(f64, f64)>);

    if columns.is_empty() && rail.is_empty() {
        return rsx! {
            div { class: "text-sm opacity-60", "No active Slices" }
        };
    }

    let rects = node_rects.read().clone();
    let origin_val = origin();
    let rail_margin = if columns.is_empty() { 0 } else { RAIL_GAP };

    rsx! {
        div { class: "overflow-x-auto flex items-stretch",
            div {
                class: "relative inline-flex items-start shrink-0",
                style: "gap: {COLUMN_GAP}px;",
                onmounted: move |evt| {
                    spawn(async move {
                        if let Ok(r) = evt.get_client_rect().await {
                            origin.set(Some((r.origin.x, r.origin.y)));
                        }
                    });
                },
                // Edge overlay: fills the row of columns; drawn from measured
                // anchors so it lines up with the real cards.
                svg {
                    class: "absolute inset-0 w-full h-full pointer-events-none overflow-visible",
                    defs {
                        marker {
                            id: "lane-arrow",
                            marker_width: "9",
                            marker_height: "7",
                            ref_x: "8",
                            ref_y: "3.5",
                            orient: "auto",
                            marker_units: "userSpaceOnUse",
                            polygon { class: "fill-base-content/60", points: "0 0, 9 3.5, 0 7" }
                        }
                    }
                    if let Some((ox, oy)) = origin_val {
                        for edge in edges.iter() {
                            if let (Some(&(bx, by, bw, bh)), Some(&(tx, ty, _tw, th))) =
                                (rects.get(&edge.blocker), rects.get(&edge.blocked))
                            {
                                {
                                    let from_x = bx + bw - ox;
                                    let from_y = by + bh / 2.0 - oy;
                                    let to_x = tx - ox;
                                    let to_y = ty + th / 2.0 - oy;
                                    let dx = ((to_x - from_x) / 2.0).max(20.0);
                                    let c1x = from_x + dx;
                                    let c2x = to_x - dx;
                                    rsx! {
                                        path {
                                            d: "M {from_x} {from_y} C {c1x} {from_y}, {c2x} {to_y}, {to_x} {to_y}",
                                            class: "stroke-base-content/50",
                                            "stroke-width": "2",
                                            fill: "none",
                                            marker_end: "url(#lane-arrow)",
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for column in columns.iter() {
                    div {
                        class: "flex flex-col shrink-0",
                        style: "width: {NODE_WIDTH}px; gap: {ROW_GAP}px;",
                        for slice in column.iter() {
                            div {
                                key: "{slice.number}",
                                onmounted: {
                                    let number = slice.number;
                                    move |evt: Event<MountedData>| {
                                        spawn(async move {
                                            if let Ok(r) = evt.get_client_rect().await {
                                                node_rects.write().insert(
                                                    number,
                                                    (r.origin.x, r.origin.y, r.size.width, r.size.height),
                                                );
                                            }
                                        });
                                    }
                                },
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
            // A sibling of the graph, outside its edge overlay and measurements.
            // Absent when empty, so an empty rail takes no space.
            if !rail.is_empty() {
                aside {
                    class: "flex flex-col shrink-0 border-l-2 border-dashed border-base-content/30 pl-6",
                    style: "margin-left: {rail_margin}px; gap: {ROW_GAP}px;",
                    "aria-label": "Slices off the dependency graph",
                    h4 {
                        class: "text-xs font-semibold uppercase tracking-wide opacity-60",
                        title: "Blocked, with no dependency in this lane, so they have no place on the dependency axis",
                        "Off the graph"
                    }
                    for slice in rail.iter() {
                        div { key: "{slice.number}", style: "width: {NODE_WIDTH}px;",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn prd(suggested: bool) -> PrdRef {
        PrdRef {
            number: 12,
            title: "A plan".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/12".to_string(),
            suggested,
        }
    }

    #[test]
    fn a_suggested_prd_is_told_which_labels_to_set() {
        let guidance = guidance_for(&prd(true)).expect("a guess has something to settle");

        assert!(guidance.contains("\"prd\" label"), "{guidance}");
        assert!(guidance.contains("\"not-a-prd\""), "{guidance}");
    }

    #[test]
    fn a_confident_prd_has_no_guidance() {
        assert_eq!(guidance_for(&prd(false)), None);
    }
}
