//! Root component: loads the board from the wired GitHub port and renders it,
//! with manual Refresh, a background poll, a "last updated" stamp, and summary
//! counts.

use std::time::{SystemTime, UNIX_EPOCH};

use dioxus::prelude::*;
use domain::{format_last_updated, BoardSummary, PollInterval, Slice, SliceState};

use crate::components::{state_badge_class, state_label, BoardColumn, ErrorBanner, SummaryBar};
use crate::state::Boot;

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    let boot = use_context::<Boot>();
    let poll = use_context::<PollInterval>();

    // Seconds-since-epoch of the most recent completed refresh, formatted for
    // display only once a load has finished.
    let mut last_updated = use_signal(|| None::<u64>);

    let mut board = use_resource(move || {
        let boot = boot.clone();
        async move {
            let result = match boot {
                Boot::Ready(state) => state.load_board().await,
                Boot::Failed(message) => Err(domain::AppError::unauthorized(message)),
            };
            last_updated.set(Some(now_unix_secs()));
            result
        }
    });

    // Background poll: refresh on the configured cadence (default ~60s).
    use_future(move || async move {
        loop {
            tokio::time::sleep(poll.as_duration()).await;
            board.restart();
        }
    });

    let board_value = board.read_unchecked();
    let slices = match &*board_value {
        Some(Ok(slices)) => slices.clone(),
        _ => Vec::new(),
    };
    let summary = BoardSummary::from_slices(&slices);
    let last_updated_label = last_updated().map(format_last_updated);

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
                h1 { class: "text-2xl font-bold", "Zfirot" }
            }

            SummaryBar {
                summary,
                last_updated: last_updated_label,
                poll_secs: poll.as_secs(),
                on_refresh: move |_| board.restart(),
            }

            match &*board_value {
                Some(Ok(slices)) => rsx! {
                    Board { slices: slices.clone() }
                },
                Some(Err(error)) => rsx! {
                    ErrorBanner { message: error.to_string() }
                },
                None => rsx! {
                    span { class: "loading loading-spinner loading-lg" }
                },
            }
        }
    }
}

/// Seconds since the UNIX epoch, for the "last updated" stamp. A clock skewed
/// before the epoch is reported as `0` rather than panicking.
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The Zfirot ZF monogram: two equal-weight, hand-drawn strokes where the Z's
/// bottom bar runs through the F stem to become the F's middle arm. Drawn with
/// `currentColor` so it follows the surrounding text colour (daisyUI primary).
#[component]
fn ZfirotLogo() -> Element {
    rsx! {
        svg {
            class: "size-7 text-primary",
            view_box: "90 110 410 390",
            fill: "none",
            stroke: "currentColor",
            "stroke-width": "56",
            "stroke-linecap": "round",
            "stroke-linejoin": "round",
            g { transform: "translate(34,0) skewX(-7)",
                // Z: top bar -> diagonal -> bottom bar (extends into the F).
                path { d: "M 138,156 Q 216,146 292,152 Q 224,250 150,338 Q 300,348 452,334" }
                // F: stem + top bar (middle arm is the Z's bottom bar).
                path { d: "M 352,188 Q 345,314 350,440 M 350,188 Q 406,182 462,194" }
            }
        }
    }
}

#[component]
fn Board(slices: Vec<Slice>) -> Element {
    rsx! {
        div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
            for state in SliceState::BOARD {
                BoardColumn {
                    state,
                    label: state_label(state).to_string(),
                    badge_class: state_badge_class(state).to_string(),
                    slices: slices.iter().filter(|s| s.state == state).cloned().collect::<Vec<_>>(),
                    on_assign: move |_number| {}, // Assign-self is wired in a later slice. No-op for now.,
                }
            }
        }
    }
}
