use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::{state_badge_class, state_label};

/// A card for a single Slice. Emits `on_assign` with the issue number when the
/// user clicks "Assign me" (only shown for Ready Slices).
#[component]
pub fn SliceCard(slice: Slice, on_assign: EventHandler<u64>) -> Element {
    let number = slice.number;
    let is_ready = slice.state == SliceState::Ready;

    rsx! {
        div { class: "card card-compact bg-base-200 shadow-sm",
            div { class: "card-body",
                div { class: "flex items-start justify-between gap-2",
                    h3 { class: "card-title text-sm", "#{slice.number} {slice.title}" }
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
            }
        }
    }
}
