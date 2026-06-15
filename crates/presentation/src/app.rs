//! Root component: gates the board behind a stored Personal Access Token.
//!
//! On launch the stored token (OS secure store) is resolved. With a token the
//! real board loads; without one the paste-token screen is shown. Saving a valid
//! token persists it and loads the board, while a rejected token surfaces an
//! inline, client-safe error.

use application::AuthService;
use dioxus::prelude::*;
use domain::{AppErrorKind, Slice, SliceState};
use infrastructure::KeyringSecureStore;

use crate::components::{state_badge_class, state_label, BoardColumn, ErrorBanner, TokenScreen};
use crate::state::AppState;

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// What the root renders once the stored token has been resolved.
enum View {
    /// A token is stored and the board loaded.
    Board(Vec<Slice>),
    /// No token stored yet: route the user to the paste-token screen.
    NeedToken,
    /// A token is stored but loading the board failed (rejected token, network).
    Error(String),
}

#[component]
pub fn App() -> Element {
    // Cleared on success, set to a client-safe message when a token is rejected.
    let mut token_error = use_signal(|| Option::<String>::None);
    // Bumped after a successful save so the token re-resolves and the board
    // takes over from the token screen.
    let mut reload = use_signal(|| 0u32);

    let view = use_resource(move || async move {
        let _ = reload(); // subscribe so a saved token re-resolves the view
        resolve_view().await
    });

    let on_submit = move |raw: String| {
        spawn(async move {
            let auth = AuthService::new(KeyringSecureStore::new());
            match auth.save_token(&raw).await {
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

        match &*view.read_unchecked() {
            // No token yet: show the paste-token screen.
            Some(View::NeedToken) => rsx! {
                TokenScreen { error: token_error(), on_submit }
            },
            // Token present and the board loaded.
            Some(View::Board(slices)) => rsx! {
                BoardShell { Board { slices: slices.clone() } }
            },
            // Token present but loading failed.
            Some(View::Error(message)) => rsx! {
                BoardShell { ErrorBanner { message: message.clone() } }
            },
            None => rsx! {
                div { class: "min-h-screen bg-base-200 grid place-items-center",
                    span { class: "loading loading-spinner loading-lg" }
                }
            },
        }
    }
}

/// Resolve the stored token and load the board, mapping the outcome to a [`View`].
///
/// A missing token (`Unauthorized`) routes to the paste-token screen; any other
/// failure is shown as a client-safe error.
async fn resolve_view() -> View {
    let auth = AuthService::new(KeyringSecureStore::new());
    let token = match auth.require_token().await {
        Ok(token) => token,
        Err(error) if error.kind() == AppErrorKind::Unauthorized => return View::NeedToken,
        Err(error) => return View::Error(error.to_string()),
    };

    let state = match AppState::from_token(&token) {
        Ok(state) => state,
        Err(error) => return View::Error(error.to_string()),
    };

    match state.load_board().await {
        Ok(slices) => View::Board(slices),
        Err(error) => View::Error(error.to_string()),
    }
}

/// The board chrome (header + logo) wrapping either the columns or an error.
#[component]
fn BoardShell(children: Element) -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
                h1 { class: "text-2xl font-bold", "Zfirot" }
            }
            {children}
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
