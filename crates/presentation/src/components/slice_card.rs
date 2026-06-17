use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::{state_badge_class, state_label};

/// A card for a single Slice. Emits `on_assign` with the issue number when the
/// user clicks "Assign me" (only shown for Ready Slices).
///
/// Dependency badges live at the bottom: a **Blocked** card lists its blockers,
/// any other card lists the issues it **unblocks**. Each badge links to its
/// GitHub issue, shows that issue's title as a tooltip, and, on hover or
/// keyboard focus, emits `on_highlight` with that issue number so the board can
/// highlight the referenced card in another column. The card highlights itself
/// when `highlighted` matches its own number.
#[component]
pub fn SliceCard(
    slice: Slice,
    on_assign: EventHandler<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    let number = slice.number;
    let is_ready = slice.state == SliceState::Ready;
    let is_highlighted = highlighted == Some(number);

    // Blocked cards surface their blockers; every other card surfaces what it
    // unblocks. An empty list renders no badge row.
    let (deps_label, deps) = if slice.state == SliceState::Blocked {
        ("Blocked by", slice.blockers.clone())
    } else {
        ("Unblocks", slice.unblocks.clone())
    };

    let card_class = if is_highlighted {
        "card card-sm bg-base-200 shadow-sm ring-2 ring-primary"
    } else {
        "card card-sm bg-base-200 shadow-sm"
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
                if !deps.is_empty() {
                    div { class: "flex flex-wrap items-center gap-1 mt-2",
                        span { class: "text-xs opacity-60", "{deps_label}:" }
                        for dep in deps {
                            a {
                                key: "{dep.number}",
                                class: "tooltip tooltip-top badge badge-sm badge-outline link link-hover",
                                "data-tip": "{dep.title}",
                                href: "{dep.url}",
                                onmouseenter: {
                                    let n = dep.number;
                                    move |_| on_highlight.call(Some(n))
                                },
                                onmouseleave: move |_| on_highlight.call(None),
                                onfocusin: {
                                    let n = dep.number;
                                    move |_| on_highlight.call(Some(n))
                                },
                                onfocusout: move |_| on_highlight.call(None),
                                "#{dep.number}"
                            }
                        }
                    }
                }
            }
        }
    }
}
