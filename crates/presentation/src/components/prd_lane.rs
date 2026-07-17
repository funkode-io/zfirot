use dioxus::prelude::*;
use domain::{AgentRef, PrdRef, Slice, SliceState};

use super::{state_badge_class, state_label, BoardColumn};

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
    agents: Vec<AgentRef>,
    on_assign: EventHandler<u64>,
    on_assign_agent: EventHandler<(u64, AgentRef)>,
    delegating: Option<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    let total_slices = slices.len();

    // Bucket each Slice into its board column exactly once, so a Slice is cloned
    // at most once per render regardless of how many columns there are.
    let mut buckets: Vec<(SliceState, Vec<Slice>)> = SliceState::BOARD
        .iter()
        .map(|&state| (state, Vec::new()))
        .collect();
    for slice in slices {
        if let Some((_, bucket)) = buckets.iter_mut().find(|(state, _)| *state == slice.state) {
            bucket.push(slice);
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
                        span { class: "badge badge-sm badge-outline badge-neutral shrink-0", "{slices_pill_label(total_slices)}" }
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
                div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                    for (state , bucket) in buckets {
                        BoardColumn {
                            state,
                            label: state_label(state).to_string(),
                            slices: bucket,
                            agents: agents.clone(),
                            on_assign,
                            on_assign_agent,
                            delegating,
                            highlighted,
                            on_highlight,
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
