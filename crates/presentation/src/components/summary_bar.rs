use dioxus::prelude::*;
use domain::BoardSummary;

/// The board's freshness and at-a-glance counts: Ready / WIP / Blocked totals,
/// a "last updated" timestamp, the poll cadence, and a manual Refresh button.
///
/// Callback-only: it takes its data as props and emits `on_refresh`, so it can
/// be previewed and tested without GitHub.
#[component]
pub fn SummaryBar(
    summary: BoardSummary,
    last_updated: Option<String>,
    poll_secs: u64,
    on_refresh: EventHandler<()>,
) -> Element {
    let last_updated = last_updated.unwrap_or_else(|| "—".to_string());

    rsx! {
        div { class: "flex flex-wrap items-center justify-between gap-3 mb-4",
            div { class: "flex items-center gap-2",
                span { class: "badge badge-success gap-1",
                    "Ready: {summary.ready}"
                }
                span { class: "badge badge-warning gap-1",
                    "WIP: {summary.wip}"
                }
                span { class: "badge badge-error gap-1",
                    "Blocked: {summary.blocked}"
                }
            }
            div { class: "flex items-center gap-3",
                span { class: "text-xs opacity-70",
                    "Last updated: {last_updated} · every {poll_secs}s"
                }
                button {
                    class: "btn btn-sm btn-primary",
                    onclick: move |_| on_refresh.call(()),
                    "Refresh"
                }
            }
        }
    }
}
