use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::{state_badge_class, state_label, DependencyBadge};

/// A card for a single Slice.
///
/// Callback-only: it emits `on_assign` with the issue number when the user
/// clicks "Assign me" (only shown for Ready Slices), and `on_hover` with a
/// referenced issue number when a dependency badge is hovered (and `None` on
/// leave) so the board can highlight the matching card in another column. The
/// `highlighted` prop is the board's currently-highlighted issue number; the
/// card lights up when it matches its own.
#[component]
pub fn SliceCard(
    slice: Slice,
    highlighted: Option<u64>,
    on_assign: EventHandler<u64>,
    on_hover: EventHandler<Option<u64>>,
) -> Element {
    let number = slice.number;
    let is_ready = slice.state == SliceState::Ready;
    let is_highlighted = highlighted == Some(slice.number);

    let card_class = if is_highlighted {
        "card card-compact bg-base-200 shadow-sm ring-2 ring-primary"
    } else {
        "card card-compact bg-base-200 shadow-sm"
    };

    rsx! {
        div { class: "{card_class}",
            div { class: "card-body",
                div { class: "flex items-start justify-between gap-2",
                    h3 { class: "card-title text-sm",
                        a { class: "link link-hover", href: "{slice.url}",
                            "#{slice.number} {slice.title}"
                        }
                    }
                    span { class: "badge badge-sm {state_badge_class(slice.state)}",
                        "{state_label(slice.state)}"
                    }
                }
                if let Some(prd) = slice.prd_title.clone() {
                    div { class: "text-xs opacity-70", "PRD: {prd}" }
                }
                div { class: "card-actions justify-between items-center mt-2",
                    if let Some(assignee) = slice.assignee.clone() {
                        span { class: "text-xs opacity-80", "@{assignee}" }
                    } else {
                        span {}
                    }
                    if is_ready {
                        button {
                            class: "btn btn-xs btn-primary",
                            onclick: move |_| on_assign.call(number),
                            "Assign me"
                        }
                    }
                }
                if !slice.blockers.is_empty() {
                    div { class: "flex flex-wrap items-center gap-1 mt-2",
                        span { class: "text-xs opacity-70", "Blocked by:" }
                        for reference in slice.blockers.clone() {
                            DependencyBadge {
                                key: "blocker-{reference.number}",
                                reference,
                                on_hover: move |n| on_hover.call(n),
                            }
                        }
                    }
                }
                if !slice.unblocks.is_empty() {
                    div { class: "flex flex-wrap items-center gap-1 mt-1",
                        span { class: "text-xs opacity-70", "Unblocks:" }
                        for reference in slice.unblocks.clone() {
                            DependencyBadge {
                                key: "unblocks-{reference.number}",
                                reference,
                                on_hover: move |n| on_hover.call(n),
                            }
                        }
                    }
                }
            }
        }
    }
}
