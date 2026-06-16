use application::OtherIssue;
use dioxus::prelude::*;
use domain::IssueClassification;

/// Card for a single issue in the "other open issues" bucket.
///
/// Renders the issue number and title, plus a "looks like a PRD/Slice — confirm?"
/// badge when the classification is [`IssueClassification::SuggestedPrd`] or
/// [`IssueClassification::SuggestedSlice`]. For those suggestions a Confirm
/// button emits `on_confirm` with the issue number and classification, so the
/// board adds the corresponding `prd`/`slice` label and re-polls. Unclassified
/// issues render without a badge or action.
#[component]
pub fn OtherIssueCard(
    issue: OtherIssue,
    on_confirm: EventHandler<(u64, IssueClassification)>,
) -> Element {
    let suggestion_badge = match issue.classification {
        IssueClassification::SuggestedPrd => Some("looks like a PRD — confirm?"),
        IssueClassification::SuggestedSlice => Some("looks like a Slice — confirm?"),
        _ => None,
    };
    // The label a confirm would add (`prd`/`slice`), or `None` when there is
    // nothing to confirm — which also gates the Confirm button.
    let confirm_label = issue.classification.suggested_label();
    let number = issue.number;
    let classification = issue.classification.clone();

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
                        div { class: "flex items-center gap-2",
                            span { class: "badge badge-sm badge-info whitespace-nowrap",
                                "{badge_text}"
                            }
                            if let Some(label) = confirm_label {
                                button {
                                    class: "btn btn-xs btn-primary",
                                    title: "Add the \"{label}\" label",
                                    "aria-label": "Confirm as {label}",
                                    onclick: {
                                        let classification = classification.clone();
                                        move |_| on_confirm.call((number, classification.clone()))
                                    },
                                    "Confirm"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
