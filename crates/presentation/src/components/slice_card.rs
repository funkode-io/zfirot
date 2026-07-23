use dioxus::prelude::*;
use domain::{Slice, SliceState};

use super::{
    pr_headline_color, pr_headline_icon_class, pr_headline_label, state_badge_class, state_label,
};

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
    let is_blocked = slice.state == SliceState::Blocked;
    // With more than one open PR, each badge shows its own status + Decorations
    // so it is obvious which redundant PR to close; a lone PR badge stays plain.
    let has_multiple_prs = slice.linked_prs.len() > 1;
    let is_highlighted = highlighted == Some(number);

    // Blocked cards surface their blockers; every other card surfaces what it
    // unblocks. An empty list renders no badge row.
    let (deps_label, deps) = if slice.state == SliceState::Blocked {
        ("Blocked by", slice.blockers.clone())
    } else {
        ("Unblocks", slice.unblocks.clone())
    };

    let card_class = if is_highlighted {
        "card card-sm bg-base-300 shadow-sm ring-2 ring-inset ring-primary py-1.5 px-3"
    } else {
        "card card-sm bg-base-300 shadow-sm py-1.5 px-3"
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
                if let Some(assignee) = slice.assignee.as_deref() {
                    // The visible "@handle" already names the assignee, so the
                    // avatar is decorative: empty alt keeps screen readers from
                    // announcing the same person twice.
                    if let Some(avatar_url) = slice.assignee_avatar_url.as_deref() {
                        div { class: "avatar",
                            div { class: "w-5 rounded-full",
                                img { src: "{avatar_url}", alt: "" }
                            }
                        }
                    }
                    span { class: "text-xs text-base-content/60","@{assignee}" }
                } else {
                    // Ready cards reserve space for the action buttons on this row.
                    div { class: "flex items-center gap-1 ml-auto",
                        if is_ready {
                            button {
                                class: "btn btn-xs btn-primary no-hover",
                                onclick: move |_| on_assign.call(number),
                                "Assign me"
                            }
                        }
                    }
                }
            }

            // --- PR status headline: the Slice's review-lifecycle spine, driven by its Best PR.
            // Shown whenever the Slice has at least one open PR; decorations ride on top.
                div { class: "flex items-center gap-1.5 mt-1.5 {pr_headline_color(pr)}",
                    span { class: "{pr_headline_icon_class(pr)} size-4 shrink-0" }
                    span { class: "text-xs font-medium", "{pr_headline_label(pr)}" }
                    // Merge-health Decorations ride on top of the status, orthogonally.
                    if pr.conflicts {
                        span {
                            class: "icon-[octicon--alert-16] text-warning size-4 shrink-0",
                            title: "Conflicts with base branch — needs a merge",
                        }
                    }
                    if pr.ci_failing {
                        span {
                            class: "icon-[octicon--x-circle-fill-16] text-error size-4 shrink-0",
                            title: "CI failing",
                        }
                    }
                    if pr.unresolved_comment_count > 0 {
                        span { class: "flex items-center gap-0.5 text-base-content/60 shrink-0",
                            title: "{pr.unresolved_comment_count} unresolved comments",
                            span { class: "icon-[octicon--comment-discussion-16] size-4" }
                            span { class: "text-xs", "{pr.unresolved_comment_count}" }
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
                            class: "badge badge-sm badge-outline link link-hover no-underline hover:underline",
                            title: "{pr.title}",
                            href: "{pr.url}",
                            if is_blocked {
                                span { class: "icon-[lucide--triangle-alert] text-warning size-3" }
                            }
                            if has_multiple_prs {
                                span { class: "{pr_headline_icon_class(pr)} size-3.5 shrink-0" }
                            }
                            if let Some(author) = &pr.author {
                                "pr #{pr.number} @{author}"
                            } else {
                                "pr #{pr.number}"
                            }
                            if has_multiple_prs {
                                if pr.conflicts {
                                    span {
                                        class: "icon-[octicon--alert-16] text-warning size-3.5 shrink-0",
                                        title: "Conflicts with base branch — needs a merge",
                                    }
                                }
                                if pr.ci_failing {
                                    span {
                                        class: "icon-[octicon--x-circle-fill-16] text-error size-3.5 shrink-0",
                                        title: "CI failing",
                                    }
                                }
                                if pr.unresolved_comment_count > 0 {
                                    span {
                                        class: "icon-[octicon--comment-discussion-16] size-3.5 shrink-0",
                                        title: "{pr.unresolved_comment_count} unresolved comments",
                                    }
                                }
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
                            class: "badge badge-sm badge-outline link link-hover no-underline hover:underline",
                            title: "{dep.title}",
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
