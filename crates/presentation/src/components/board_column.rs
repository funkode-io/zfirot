use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::SliceCard;

/// A single board column for one [`SliceState`], listing its Slices.
///
/// `highlighted` is the board's currently-highlighted issue number, passed to
/// every card so a hovered dependency badge in another column lights up the
/// matching card here. `on_hover` forwards each card's hover intent back up to
/// the board, which owns the shared highlight state.
#[component]
pub fn BoardColumn(
    state: SliceState,
    label: String,
    badge_class: String,
    slices: Vec<Slice>,
    highlighted: Option<u64>,
    on_assign: EventHandler<u64>,
    on_hover: EventHandler<Option<u64>>,
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
                        highlighted,
                        on_assign: move |number| on_assign.call(number),
                        on_hover: move |number| on_hover.call(number),
                    }
                }
            }
        }
    }
}
