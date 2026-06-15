use dioxus::prelude::*;
use domain::{PrdRef, Slice, SliceState};

use super::{state_badge_class, state_label, BoardColumn};

/// One swimlane: a collapsible PRD header above the Ready / WIP / Blocked
/// columns holding that PRD's Slices. A lane with no PRD (`prd` is `None`)
/// renders a plain "No PRD" header. Collapsing the lane hides the columns and
/// summarises them as coloured per-state count badges; the state name is shown
/// on hover via each badge's `title`.
#[component]
pub fn PrdLane(prd: Option<PrdRef>, slices: Vec<Slice>, on_assign: EventHandler<u64>) -> Element {
    let mut collapsed = use_signal(|| false);

    // Per-state counts for the collapsed summary, in board column order.
    let counts: Vec<(SliceState, usize)> = SliceState::BOARD
        .iter()
        .map(|&state| (state, slices.iter().filter(|s| s.state == state).count()))
        .collect();

    rsx! {
        section { class: "bg-base-200 rounded-box p-4",
            div { class: "flex items-center gap-3 mb-3",
                button {
                    class: "btn btn-ghost btn-xs btn-square",
                    "aria-label": if collapsed() { "Expand lane" } else { "Collapse lane" },
                    onclick: move |_| collapsed.set(!collapsed()),
                    span { class: if collapsed() { "icon-[lucide--chevron-right]" } else { "icon-[lucide--chevron-down]" } }
                }
                match prd {
                    Some(prd) => rsx! {
                        a { class: "link link-hover font-semibold", href: "{prd.url}", "#{prd.number} {prd.title}" }
                    },
                    None => rsx! {
                        span { class: "font-semibold opacity-70", "No PRD" }
                    },
                }
                if collapsed() {
                    div { class: "flex items-center gap-1 ml-auto",
                        for (state , count) in counts.iter().copied() {
                            span {
                                class: "badge badge-sm {state_badge_class(state)}",
                                title: "{state_label(state)}",
                                "{count}"
                            }
                        }
                    }
                }
            }
            if !collapsed() {
                div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                    for state in SliceState::BOARD {
                        BoardColumn {
                            state,
                            label: state_label(state).to_string(),
                            badge_class: state_badge_class(state).to_string(),
                            slices: slices.iter().filter(|s| s.state == state).cloned().collect::<Vec<_>>(),
                            on_assign: move |number| on_assign.call(number),
                        }
                    }
                }
            }
        }
    }
}
