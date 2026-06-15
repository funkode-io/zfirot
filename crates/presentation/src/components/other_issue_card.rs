use application::OtherIssue;
use dioxus::prelude::*;
use domain::IssueClassification;

/// Card for a single issue in the "other open issues" bucket.
///
/// Renders the issue number and title, plus a "looks like a PRD/Slice — confirm?"
/// badge when the classification is [`IssueClassification::SuggestedPrd`] or
/// [`IssueClassification::SuggestedSlice`]. No write action is performed here;
/// the confirm-and-label action arrives in a later slice.
#[component]
pub fn OtherIssueCard(issue: OtherIssue) -> Element {
    let suggestion_badge = match issue.classification {
        IssueClassification::SuggestedPrd => Some("looks like a PRD — confirm?"),
        IssueClassification::SuggestedSlice => Some("looks like a Slice — confirm?"),
        _ => None,
    };

    rsx! {
        div { class: "card card-compact bg-base-200 shadow-sm",
            div { class: "card-body",
                div { class: "flex items-start justify-between gap-2",
                    h3 { class: "card-title text-sm",
                        a { class: "link link-hover", href: "{issue.url}",
                            "#{issue.number} {issue.title}"
                        }
                    }
                    if let Some(badge_text) = suggestion_badge {
                        span { class: "badge badge-sm badge-info whitespace-nowrap",
                            "{badge_text}"
                        }
                    }
                }
            }
        }
    }
}
