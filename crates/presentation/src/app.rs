//! Root component: loads the board (via a fake port for now) and renders it.

use application::BoardService;
use dioxus::prelude::*;
use domain::{RepoRef, Slice, SliceState};
use infrastructure::FakeGitHubPort;

use crate::components::{state_badge_class, state_label, BoardColumn};

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    let board = use_resource(|| async {
        let service = BoardService::new(FakeGitHubPort);
        let repo = RepoRef::new("funkode-io", "zfirot");
        service.load_board(&repo).await
    });

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                span { class: "icon-[lucide--layout-dashboard] size-7" }
                h1 { class: "text-2xl font-bold", "Zfirot" }
            }

            match &*board.read_unchecked() {
                Some(Ok(slices)) => rsx! {
                    Board { slices: slices.clone() }
                },
                Some(Err(error)) => rsx! {
                    div { class: "alert alert-error", "{error}" }
                },
                None => rsx! {
                    span { class: "loading loading-spinner loading-lg" }
                },
            }
        }
    }
}

#[component]
fn Board(slices: Vec<Slice>) -> Element {
    rsx! {
        div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
            for state in SliceState::ALL {
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
