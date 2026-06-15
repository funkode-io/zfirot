use dioxus::prelude::*;
use domain::{PrdRef, Slice, SliceState};

use super::{state_badge_class, state_label, BoardColumn};

/// A board swimlane for one PRD (or the trailing "No PRD" lane), containing the
/// Ready / WIP / Blocked state columns for that PRD's Slices.
///
/// The lane header links to the PRD issue on GitHub when a PRD is known. This is
/// a callback-only layout component: it groups already-loaded Slices and relays
/// `on_assign` up, never touching the application or GitHub itself.
#[component]
pub fn PrdLane(prd: Option<PrdRef>, slices: Vec<Slice>, on_assign: EventHandler<u64>) -> Element {
    rsx! {
        section { class: "bg-base-100 rounded-box p-4",
            div { class: "mb-3",
                match prd {
                    Some(prd) => rsx! {
                        h2 { class: "text-lg font-bold",
                            a { class: "link link-hover", href: "{prd.url}",
                                "#{prd.number} {prd.title}"
                            }
                        }
                    },
                    None => rsx! {
                        h2 { class: "text-lg font-bold opacity-70", "No PRD" }
                    },
                }
            }
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
