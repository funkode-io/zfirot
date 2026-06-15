//! Root component: gates the board behind a stored Personal Access Token. When
//! none is saved it shows the paste-token screen; once a token is in the OS
//! secure store the board loads (via a fake port for now).

use application::{AuthService, BoardService};
use dioxus::prelude::*;
use domain::{RepoRef, Slice, SliceState};
use infrastructure::{FakeGitHubPort, KeyringSecureStore};

use crate::components::{state_badge_class, state_label, BoardColumn, TokenScreen};

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    // Cleared on success, set to a client-safe message when a token is rejected.
    let mut token_error = use_signal(|| Option::<String>::None);
    // Bumped after a successful save so the auth check re-runs and the board
    // takes over from the token screen.
    let mut reload = use_signal(|| 0u32);

    let auth = use_resource(move || async move {
        let _ = reload(); // subscribe so a saved token re-checks
        let service = AuthService::new(KeyringSecureStore::new());
        service.has_token().await
    });

    let on_submit = move |raw: String| {
        spawn(async move {
            let service = AuthService::new(KeyringSecureStore::new());
            match service.save_token(&raw).await {
                Ok(()) => {
                    token_error.set(None);
                    reload += 1;
                }
                Err(error) => token_error.set(Some(error.to_string())),
            }
        });
    };

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        match &*auth.read_unchecked() {
            // A token is stored: load the board with it.
            Some(Ok(true)) => rsx! { BoardView {} },
            // No token yet: route the user to the paste-token screen.
            Some(Ok(false)) => rsx! {
                TokenScreen { error: token_error(), on_submit }
            },
            Some(Err(error)) => rsx! {
                div { class: "min-h-screen bg-base-200 p-6",
                    div { class: "alert alert-error", "{error}" }
                }
            },
            None => rsx! {
                div { class: "min-h-screen bg-base-200 grid place-items-center",
                    span { class: "loading loading-spinner loading-lg" }
                }
            },
        }
    }
}

/// The board screen, shown once a token is stored. Loads the Slices (via a fake
/// port for now) and renders the Kanban columns.
#[component]
fn BoardView() -> Element {
    let board = use_resource(|| async {
        let service = BoardService::new(FakeGitHubPort);
        let repo = RepoRef::new("funkode-io", "zfirot");
        service.load_board(&repo).await
    });

    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
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
