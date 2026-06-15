use dioxus::prelude::*;
use domain::IssueRef;

/// A clickable dependency badge for a single related issue.
///
/// Callback-only: it links to the referenced issue on GitHub and emits hover
/// intents so the board can highlight the matching card in another column. It
/// reports the referenced issue number on hover-enter and `None` on hover-leave;
/// it never reaches for any state of its own.
#[component]
pub fn DependencyBadge(reference: IssueRef, on_hover: EventHandler<Option<u64>>) -> Element {
    let number = reference.number;

    rsx! {
        a {
            class: "badge badge-sm badge-outline link link-hover",
            href: "{reference.url}",
            onmouseenter: move |_| on_hover.call(Some(number)),
            onmouseleave: move |_| on_hover.call(None),
            "#{number}"
        }
    }
}
