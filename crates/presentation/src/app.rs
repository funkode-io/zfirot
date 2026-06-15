//! Root component: gates the board behind a stored Personal Access Token.
//!
//! On launch the stored token (OS secure store) is resolved. With a token the
//! real board loads; without one the paste-token screen is shown. Saving a valid
//! token persists it and loads the board. A stored token that GitHub rejects is
//! discarded and the user is routed back to the paste-token screen to enter a
//! new one, with the reason shown inline.

use application::{AuthService, SecureStorePort};
use dioxus::prelude::*;
use domain::{AppErrorKind, GitHubToken, Project, RepoRef, Slice, SliceState};

use crate::components::{
    state_badge_class, state_label, BoardColumn, ErrorBanner, HomeScreen, TokenScreen,
};
use crate::state::{last_opened, open_project, recent_projects, secure_store, AppState};

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// What the root renders once the stored token has been resolved.
enum View {
    /// A token is stored but no project is open yet: pick from recent projects.
    Home(Vec<Project>),
    /// A token is stored and the board for `repo` loaded.
    Board { repo: RepoRef, slices: Vec<Slice> },
    /// Show the paste-token screen. `reason` is `Some` when a stored token was
    /// rejected by GitHub (so the user knows why they are being asked again) and
    /// `None` on first launch when no token has ever been saved.
    NeedToken { reason: Option<String> },
    /// A token is stored but loading failed for a non-auth reason (network, rate
    /// limit): a transient error shown in the board shell.
    Error(String),
}

/// Where the user wants to be, independent of what is persisted on disk.
#[derive(Clone, PartialEq)]
enum Nav {
    /// Initial launch: reopen the last-opened project, else show the home screen.
    Auto,
    /// Explicitly show the home screen (the recent-projects picker).
    Home,
    /// Show the board for a project the user just chose.
    Project(RepoRef),
}

#[component]
pub fn App() -> Element {
    // Cleared on success, set to a client-safe message when a token is rejected.
    let mut token_error = use_signal(|| Option::<String>::None);
    // Bumped after a successful save or project selection so the view re-resolves.
    let mut reload = use_signal(|| 0u32);
    // Where the user wants to be; starts at `Auto` so the last-opened project
    // reopens on launch, and switches to `Home`/`Project` as they navigate.
    let mut nav = use_signal(|| Nav::Auto);

    let view = use_resource(move || async move {
        let _ = reload(); // subscribe so a save or selection re-resolves the view
        resolve_view(nav()).await
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

    let on_open = move |repo: RepoRef| {
        spawn(async move {
            // Persist the choice (best-effort) and navigate to its board.
            let _ = open_project(&repo).await;
            nav.set(Nav::Project(repo));
            reload += 1;
        });
    };

    // Back to the project picker. Persistence is untouched, so the next launch
    // still reopens the last project; this only changes the current session.
    let on_home = move |_| {
        nav.set(Nav::Home);
        reload += 1;
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
            // Token present but no project open: show recent projects.
            Some(View::Home(projects)) => rsx! {
                HomeScreen { projects: projects.clone(), on_open }
            },
            // Token present and the board loaded.
            Some(View::Board { repo, slices }) => rsx! {
                BoardShell { repo: repo.to_string(), on_home,
                    Board { slices: slices.clone() }
                }
            },
            Some(View::Error(message)) => rsx! {
                BoardShell { on_home,
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

/// Resolve the stored token and decide what to show, mapping the outcome to a
/// [`View`].
///
/// A missing token (`Unauthorized` from `require_token`) routes to the paste-token
/// screen. With a token, the requested [`Nav`] decides the board: an explicit
/// project, the last-opened one (`Auto`), or the home screen (`Home`, or `Auto`
/// when nothing has been opened yet). A stored token that GitHub rejects while
/// loading (`Unauthorized`/`Forbidden`) is discarded and routes back to the
/// screen, carrying the reason. Any other failure is shown as a transient error.
async fn resolve_view(nav: Nav) -> View {
    let auth = AuthService::new(secure_store());
    let token = match auth.require_token().await {
        Ok(token) => token,
        Err(error) if error.kind() == AppErrorKind::Unauthorized => {
            return View::NeedToken { reason: None }
        }
        Err(error) => return View::Error(error.to_string()),
    };

    // Decide the project to open from where the user wants to be. `Home` always
    // shows the picker; `Auto` reopens the last-opened project or, failing that,
    // shows the picker too.
    let repo = match nav {
        Nav::Home => return home_view(&auth, &token).await,
        Nav::Project(repo) => repo,
        Nav::Auto => match last_opened().await {
            Ok(Some(repo)) => repo,
            Ok(None) => return home_view(&auth, &token).await,
            Err(error) => return View::Error(error.to_string()),
        },
    };

    let state = match AppState::from_token(&token, repo.clone()) {
        Ok(state) => state,
        Err(error) => return View::Error(error.to_string()),
    };

    match state.load_board().await {
        Ok(slices) => View::Board { repo, slices },
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

/// The home screen's recent-projects view. A token GitHub rejects while listing
/// (`Unauthorized`/`Forbidden`) is discarded and routes back to the paste-token
/// screen, just like the board path; any other failure is a transient error.
async fn home_view<S: SecureStorePort>(auth: &AuthService<S>, token: &GitHubToken) -> View {
    match recent_projects(token).await {
        Ok(projects) => View::Home(projects),
        Err(error) if is_auth_failure(error.kind()) => {
            // A rejected token must not strand the user on an error screen.
            // Discard it (best-effort) and route back to paste a new one.
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
/// `repo` names the open project (shown beside the title) when there is one;
/// `on_home` returns to the project picker.
#[component]
fn BoardShell(
    children: Element,
    on_home: EventHandler<()>,
    #[props(default)] repo: Option<String>,
) -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
                h1 { class: "text-2xl font-bold", "Zfirot" }
                if let Some(repo) = repo {
                    span { class: "text-base opacity-60", "/ {repo}" }
                    button {
                        class: "btn btn-ghost btn-sm btn-square",
                        title: "Back to projects",
                        onclick: move |_| on_home.call(()),
                        span { class: "icon-[lucide--undo-2] size-5" }
                    }
                }
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
