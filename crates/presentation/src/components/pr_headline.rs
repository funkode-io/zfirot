use dioxus::prelude::*;
use domain::LinkedPrRef;

use super::{pr_headline_color, pr_headline_icon_class, pr_headline_label};

/// A PR's review-lifecycle headline, with its merge-health Decorations riding on
/// top of the status, orthogonally.
#[component]
pub fn PrHeadline(pr: LinkedPrRef) -> Element {
    rsx! {
        div { class: "flex items-center gap-1.5 mt-1.5 {pr_headline_color(&pr)}",
            span { class: "{pr_headline_icon_class(&pr)} size-4 shrink-0" }
            span { class: "text-xs font-medium", "{pr_headline_label(&pr)}" }
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
}
