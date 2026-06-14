use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::SliceCard;

/// A single board column for one [`SliceState`], listing its Slices.
#[component]
pub fn BoardColumn(
    state: SliceState,
    label: String,
    badge_class: String,
    slices: Vec<Slice>,
    on_assign: EventHandler<u64>,
) -> Element {
    rsx! {
        div { class: "bg-base-100 rounded-box p-3",
            div { class: "flex items-center justify-between mb-3",
                h2 { class: "font-semibold", "{label}" }
                span { class: "badge {badge_class}", "{slices.len()}" }
            }
            div { class: "flex flex-col gap-2",
                for slice in slices {
                    SliceCard {
                        key: "{slice.number}",
                        slice: slice.clone(),
                        on_assign: move |number| on_assign.call(number),
                    }
                }
            }
        }
    }
}
