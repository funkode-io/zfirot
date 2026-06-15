use dioxus::prelude::*;
use domain::{PrdRef, Slice, SliceState};

use super::{state_badge_class, state_label, BoardColumn};

/// One swimlane: a collapsible PRD header above the Ready / WIP / Blocked
/// columns holding that PRD's Slices. A lane with no PRD (`prd` is `None`)
/// renders a plain "No PRD" header. The whole header row toggles the lane, with
/// a chevron on the right (matching the "other open issues" accordion).
/// Collapsing the lane hides the columns and summarises them as coloured
/// per-state count badges; the state name is shown on hover via each badge's
/// `title`.
#[component]
pub fn PrdLane(prd: Option<PrdRef>, slices: Vec<Slice>, on_assign: EventHandler<u64>) -> Element {
    let mut collapsed = use_signal(|| false);

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
        section { class: "bg-base-200 rounded-box p-4",
            button {
                class: "flex items-center gap-3 w-full text-left",
                "aria-label": if collapsed() { "Expand lane" } else { "Collapse lane" },
                onclick: move |_| collapsed.set(!collapsed()),
                match prd {
                    Some(prd) => rsx! {
                        a {
                            class: "link link-hover font-semibold",
                            href: "{prd.url}",
                            onclick: move |e: Event<MouseData>| e.stop_propagation(),
                            "#{prd.number} {prd.title}"
                        }
                    },
                    None => rsx! {
                        span { class: "font-semibold opacity-70", "No PRD" }
                    },
                }
                if collapsed() {
                    div { class: "flex items-center gap-1",
                        for (state , count) in counts.iter().copied() {
                            span {
                                class: "badge badge-sm {state_badge_class(state)}",
                                title: "{state_label(state)}",
                                "{count}"
                            }
                        }
                    }
                }
                span { class: if collapsed() { "icon-[lucide--chevron-down] ml-auto" } else { "icon-[lucide--chevron-up] ml-auto" } }
            }
            if !collapsed() {
                div { class: "grid grid-cols-1 md:grid-cols-3 gap-4 mt-3",
                    for (state , bucket) in buckets {
                        BoardColumn {
                            state,
                            label: state_label(state).to_string(),
                            badge_class: state_badge_class(state).to_string(),
                            slices: bucket,
                            on_assign: move |number| on_assign.call(number),
                        }
                    }
                }
            }
        }
    }
}
