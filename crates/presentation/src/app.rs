//! Root component: gates the board behind a stored Personal Access Token.
//!
//! On launch the stored token (OS secure store) is resolved. With a token the
//! real board loads; without one the paste-token screen is shown. Saving a valid
//! token persists it and loads the board. A stored token that GitHub rejects is
//! discarded and the user is routed back to the paste-token screen to enter a
//! new one, with the reason shown inline.

use application::AuthService;
use dioxus::prelude::*;
use domain::{AppErrorKind, Slice};

use crate::components::{ErrorBanner, PrdLane, TokenScreen};
use crate::state::{secure_store, AppState};

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// What the root renders once the stored token has been resolved.
enum View {
    /// A token is stored and the board loaded.
    Board(Vec<Slice>),
    /// Show the paste-token screen. `reason` is `Some` when a stored token was
    /// rejected by GitHub (so the user knows why they are being asked again) and
    /// `None` on first launch when no token has ever been saved.
    NeedToken { reason: Option<String> },
    /// A token is stored but loading the board failed for a non-auth reason
    /// (network, rate limit): a transient error shown in the board shell.
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
            let auth = AuthService::new(secure_store());
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
            // No token yet, or a stored token was rejected: show the paste-token
            // screen. A fresh submit error takes precedence over a stale reject
            // reason carried by the view.
            Some(View::NeedToken { reason }) => rsx! {
                TokenScreen { error: token_error().or_else(|| reason.clone()), on_submit }
            },
            // Token present and the board loaded.
            Some(View::Board(slices)) => rsx! {
                BoardShell {
                    Board { slices: slices.clone() } // Token present but loading failed. // Token present but loading failed.
                }
            },
            Some(View::Error(message)) => rsx! {
                BoardShell {
                    ErrorBanner { message: message.clone() }
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

/// Resolve the stored token and load the board, mapping the outcome to a [`View`].
///
/// A missing token (`Unauthorized` from `require_token`) routes to the paste-token
/// screen. A stored token that GitHub rejects while loading the board
/// (`Unauthorized`/`Forbidden`) is discarded and also routes back to the screen,
/// carrying the reason. Any other failure is shown as a transient error.
async fn resolve_view() -> View {
    let auth = AuthService::new(secure_store());
    let token = match auth.require_token().await {
        Ok(token) => token,
        Err(error) if error.kind() == AppErrorKind::Unauthorized => {
            return View::NeedToken { reason: None }
        }
        Err(error) => return View::Error(error.to_string()),
    };

    let state = match AppState::from_token(&token) {
        Ok(state) => state,
        Err(error) => return View::Error(error.to_string()),
    };

    match state.load_board().await {
        Ok(slices) => View::Board(slices),
        Err(error) if is_auth_failure(error.kind()) => {
            // The stored token was rejected (revoked, expired, or missing
            // scopes). Discard it so we do not loop on a known-bad secret, then
            // route the user back to the screen to paste a new one. Clearing is
            // best-effort: routing back is what matters.
            let _ = auth.clear_token().await;
            View::NeedToken {
                reason: Some(error.to_string()),
            }
        }
        Err(error) => View::Error(error.to_string()),
    }
}

/// Whether an error means the token itself is the problem (so the user should be
/// asked for a new one) rather than a transient/network failure.
fn is_auth_failure(kind: AppErrorKind) -> bool {
    matches!(kind, AppErrorKind::Unauthorized | AppErrorKind::Forbidden)
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
    let lanes = domain::group_into_lanes(slices);
    rsx! {
        div { class: "flex flex-col gap-6",
            for lane in lanes {
                PrdLane {
                    key: "{lane.prd.as_ref().map(|prd| prd.number).unwrap_or(0)}",
                    prd: lane.prd,
                    slices: lane.slices,
                    on_assign: move |_number| {}, // Assign-self is wired in a later slice. No-op for now.,
                }
            }
        }
    }
}
