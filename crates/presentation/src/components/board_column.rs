use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::{state_dot_color, SliceCard};

/// A single board column for one [`SliceState`], listing its Slices.
///
/// `highlighted` is the shared "highlighted issue" the whole board coordinates;
/// each card highlights itself when it matches and re-emits hover intents via
/// `on_highlight`.
#[component]
pub fn BoardColumn(
    state: SliceState,
    label: String,
    slices: Vec<Slice>,
    on_assign: EventHandler<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2",
            // GitHub Projects–style column header: status dot + label + count pill
            div { class: "flex items-center gap-2",
                span {
                    class: "{state_dot_color(state)} size-2 rounded-full shrink-0",
                    title: "{label}",
                }
                h3 { class: "font-medium text-sm leading-tight", "{label}" }
                span { class: "badge badge-sm badge-ghost", "{slices.len()}" }
            }
            // Bordered, theme-surfaced container for the cards
            div { class: "border border-base-300 bg-base-200 rounded-box p-2 flex flex-col gap-2",
                for slice in slices {
                    SliceCard {
                        key: "{slice.number}",
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
