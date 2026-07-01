use dioxus::prelude::*;
use domain::{AgentRef, Slice, SliceState};

use super::{agent_action, state_badge_class, state_label, AgentAction};

/// A card for a single Slice. Emits `on_assign` with the issue number when the
/// user clicks "Assign me" (only shown for Ready Slices).
///
/// A Ready card also carries the **adaptive Agent action** driven by the board's
/// Assignable `agents`: none → no Agent action; one → a single "Assign &lt;name&gt;"
/// button; two or more → a picker. Selecting an Agent emits `on_assign_agent`
/// with the issue number and chosen [`AgentRef`]. While `delegating` matches this
/// card's number the actions show a spinner and are disabled.
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
    agents: Vec<AgentRef>,
    on_assign: EventHandler<u64>,
    on_assign_agent: EventHandler<(u64, AgentRef)>,
    delegating: Option<u64>,
    highlighted: Option<u64>,
    on_highlight: EventHandler<Option<u64>>,
) -> Element {
    let number = slice.number;
    let is_ready = slice.state == SliceState::Ready;
    let is_blocked = slice.state == SliceState::Blocked;
    let is_highlighted = highlighted == Some(number);
    let is_delegating = delegating == Some(number);

    // Blocked cards surface their blockers; every other card surfaces what it
    // unblocks. An empty list renders no badge row.
    let (deps_label, deps) = if slice.state == SliceState::Blocked {
        ("Blocked by", slice.blockers.clone())
    } else {
        ("Unblocks", slice.unblocks.clone())
    };

    let card_class = if is_highlighted {
        "card card-sm bg-base-100 shadow-sm ring-2 ring-primary py-1.5 px-3"
    } else {
        "card card-sm bg-base-100 shadow-sm py-1.5 px-3"
    };

    rsx! {
        div { class: "{card_class}",
            // --- title row: leading icon + title-first, soft badge on the right ---
            div { class: "flex items-start justify-between gap-2",
                div { class: "flex items-center gap-2 min-w-0 flex-1",
                    span {
                        class: "icon-[lucide--circle-dot] text-base-content/50 size-4 shrink-0 mt-0.5",
                    }
                    h3 { class: "font-medium text-sm leading-snug truncate min-w-0",
                        a { class: "link link-hover link-primary no-underline hover:underline whitespace-nowrap overflow-hidden text-ellipsis",
                            href: "{slice.url}",
                            "{slice.title}"
                        }
                    }
                }
                span { class: "badge badge-sm badge-soft {state_badge_class(slice.state)} shrink-0",
                    "{state_label(slice.state)}"
                }
            }

            // --- compact meta row: number + assignee / spacer ---
            div { class: "flex items-center gap-2 mt-1.5",
                span { class: "text-xs text-base-content/50 shrink-0",
                    "#{slice.number}"
                }
                if let Some(assignee) = slice.assignee.clone() {
                    span { class: "text-xs text-base-content/60","@{assignee}" }
                } else {
                    // Ready cards reserve space for the action buttons on this row.
                    div { class: "flex items-center gap-1 ml-auto",
                        if is_ready {
                            button {
                                class: "btn btn-xs btn-primary no-hover",
                                disabled: is_delegating,
                                onclick: move |_| on_assign.call(number),
                                if is_delegating {
                                    span { class: "loading loading-spinner loading-xs" }
                                }
                                "Assign me"
                            }
                            AgentActionButtons {
                                number,
                                action: agent_action(&agents),
                                delegating: is_delegating,
                                on_assign_agent,
                            }
                        }
                    }
                }
            }

            // --- linked PR badges (all states) ---
            if !slice.linked_prs.is_empty() {
                div { class: "flex flex-wrap items-center gap-1 mt-1.5",
                    for pr in slice.linked_prs.iter() {
                        a {
                            key: "{pr.number}",
                            class: "tooltip tooltip-top badge badge-sm badge-outline link link-hover no-underline hover:underline",
                            "data-tip": "{pr.title}",
                            href: "{pr.url}",
                            if is_blocked {
                                span { class: "icon-[lucide--triangle-alert] text-warning size-3" }
                            }
                            if let Some(author) = &pr.author {
                                "pr #{pr.number} @{author}"
                            } else {
                                "pr #{pr.number}"
                            }
                        }
                    }
                }
            }

            // --- dependency badges: blockers (Blocked cards) or unblocks (all others) ---
            if !deps.is_empty() {
                div { class: "flex flex-wrap items-center gap-1 mt-1.5",
                    span { class: "text-xs text-base-content/50 shrink-0","{deps_label}" }
                    for dep in deps {
                        a {
                            key: "{dep.number}",
                            class: "tooltip tooltip-top badge badge-sm badge-outline link link-hover no-underline hover:underline",
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

/// The adaptive Agent action for a Ready card: nothing, a single labelled
/// button, or a picker dropdown — per [`agent_action`]. While `delegating` it
/// shows an in-flight spinner and disables selection.
#[component]
fn AgentActionButtons(
    number: u64,
    action: AgentAction,
    delegating: bool,
    on_assign_agent: EventHandler<(u64, AgentRef)>,
) -> Element {
    match action {
        AgentAction::None => rsx! {},
        AgentAction::Single(agent) => {
            let label = agent.name.clone();
            rsx! {
                button {
                    class: "btn btn-xs btn-secondary",
                    disabled: delegating,
                    onclick: move |_| on_assign_agent.call((number, agent.clone())),
                    if delegating {
                        span { class: "loading loading-spinner loading-xs" }
                    }
                    "Assign {label}"
                }
            }
        }
        AgentAction::Picker(agents) => rsx! {
            div { class: "dropdown dropdown-end",
                button {
                    class: "btn btn-xs btn-secondary",
                    tabindex: "0",
                    disabled: delegating,
                    if delegating {
                        span { class: "loading loading-spinner loading-xs" }
                    }
                    "Assign Agent"
                    span { class: "icon-[lucide--chevron-down]" }
                }
                if !delegating {
                    ul {
                        class: "dropdown-content menu bg-base-100 rounded-box z-10 w-44 p-2 shadow-sm",
                        tabindex: "0",
                        for agent in agents {
                            li { key: "{agent.node_id}",
                                button {
                                    onclick: {
                                        let agent = agent.clone();
                                        move |_| on_assign_agent.call((number, agent.clone()))
                                    },
                                    "{agent.name}"
                                }
                            }
                        }
                    }
                }
            }
        },
    }
}
