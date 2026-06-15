//! Root component: loads the board from the wired GitHub port and renders it.

use application::{ClassifiedBoard, OtherIssue};
use dioxus::prelude::*;
use domain::{Slice, SliceState};

use crate::components::{state_badge_class, state_label, BoardColumn, ErrorBanner, OtherIssueCard};
use crate::state::Boot;

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    let boot = use_context::<Boot>();
    let board = use_resource(move || {
        let boot = boot.clone();
        async move {
            match boot {
                Boot::Ready(state) => state.classify_board().await,
                Boot::Failed(message) => Err(domain::AppError::unauthorized(message)),
            }
        }
    });

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
                h1 { class: "text-2xl font-bold", "Zfirot" }
            }

            match &*board.read_unchecked() {
                Some(Ok(classified)) => rsx! {
                    Board { slices: classified.slices.clone() }
                    if !classified.other.is_empty() {
                        OtherIssues { issues: classified.other.clone() }
                    }
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

/// The "other open issues" bucket — shows suggested and unclassified issues
/// below the Kanban board.
///
/// Suggested issues (tier-2 classification) render with a
/// "looks like a PRD/Slice — confirm?" badge. Unclassified issues render
/// without any badge. No write action is performed here.
#[component]
fn OtherIssues(issues: Vec<OtherIssue>) -> Element {
    rsx! {
        section { class: "mt-6",
            h2 { class: "text-lg font-semibold mb-3", "Other open issues" }
            div { class: "flex flex-col gap-2",
                for issue in issues {
                    OtherIssueCard {
                        key: "{issue.number}",
                        issue: issue.clone(),
                    }
                }
            }
        }
    }
}
