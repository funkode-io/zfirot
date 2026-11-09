use dioxus::prelude::*;
use domain::LinkedPrRef;

use super::PrHeadline;

/// Card for a single **Unlinked PR**: its title links to the PR, its `pr #n @u`
/// names the PR and its author, and the headline carries its status and
/// Decorations exactly as a Slice card would.
#[component]
pub fn UnlinkedPrCard(pr: LinkedPrRef) -> Element {
    rsx! {
        div { class: "card card-sm bg-base-300 shadow-sm py-1.5 px-3",
            h3 { class: "font-medium text-sm leading-snug truncate min-w-0",
                a { class: "link link-hover link-primary no-underline hover:underline",
                    href: "{pr.url}",
                    "{pr.title}"
                }
            }
            div { class: "flex flex-wrap items-center gap-3",
                PrHeadline { pr: pr.clone() }
                span { class: "text-xs text-base-content/50 mt-1.5",
                    if let Some(author) = &pr.author {
                        "pr #{pr.number} @{author}"
                    } else {
                        "pr #{pr.number}"
                    }
                }
            }
        }
    }
}
