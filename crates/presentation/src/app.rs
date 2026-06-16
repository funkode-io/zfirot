//! Root component: gates the board behind a stored Personal Access Token.
//!
//! On launch the stored token (OS secure store) is resolved. With a token the
//! real board loads; without one the paste-token screen is shown. Saving a valid
//! token persists it and loads the board. A stored token that GitHub rejects is
//! discarded and the user is routed back to the paste-token screen to enter a
//! new one, with the reason shown inline.

use application::{AuthService, ClassifiedBoard, OtherIssue, ProjectsRefresh, SecureStorePort};
use dioxus::prelude::*;
use domain::{
    group_into_lanes, AppErrorKind, BoardSummary, GitHubToken, IssueClassification, PollInterval,
    Project, RepoRef, Slice,
};

use crate::components::{
    ErrorBanner, HomeScreen, LoadingScreen, OtherIssueCard, PrdLane, Spinner, TokenScreen,
};
use crate::state::{
    assign_self, cached_projects, confirm_classification, last_opened, open_project,
    refresh_projects, refresh_recent_projects, secure_store, AppState,
};

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// Build it with `make css` (runs `npm run build:css` in crates/presentation).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// What the root renders once the stored token has been resolved.
enum View {
    /// A token is stored but no project is open yet: pick from recent projects.
    /// `from_cache` is `true` only when this list came from an instant cached
    /// paint that still needs revalidating; a list produced by a live fetch
    /// (cold-cache fallback or a completed refresh) sets it `false` so the
    /// background effect does not fetch again.
    Home {
        projects: Vec<Project>,
        from_cache: bool,
    },
    /// A token is stored and the board for `repo` loaded and was classified into
    /// confirmed Slices plus an "other open issues" bucket. `loaded_at` is the
    /// local wall-clock time this snapshot was fetched, shown as "last updated".
    Board {
        repo: RepoRef,
        board: ClassifiedBoard,
        loaded_at: String,
    },
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
    // Set to a client-safe message when assigning self to a Slice fails, shown
    // above the board; cleared on a successful assignment.
    let mut assign_error = use_signal(|| Option::<String>::None);
    // Set to a client-safe message when confirming a suggested classification
    // fails, shown above the board; cleared on a successful confirm.
    let mut confirm_error = use_signal(|| Option::<String>::None);
    // Bumped after a successful save or project selection so the view re-resolves.
    let mut reload = use_signal(|| 0u32);
    // Where the user wants to be; starts at `Auto` so the last-opened project
    // reopens on launch, and switches to `Home`/`Project` as they navigate.
    let mut nav = use_signal(|| Nav::Auto);
    // Guards the stale-while-revalidate refresh so it runs once per visit to the
    // home screen, not on every `reload` bump. Reset when the user navigates
    // back to Home so returning revalidates again.
    let mut revalidated = use_signal(|| false);
    // True while a freshly-pasted token is being validated and persisted. This
    // save is a mutation, not part of the read-only `view` resource, so it needs
    // its own in-flight flag to drive the spinner on the submit button.
    let mut saving = use_signal(|| false);

    let view = use_resource(move || async move {
        let _ = reload(); // subscribe so a save or selection re-resolves the view
        resolve_view(nav()).await
    });

    // Stale-while-revalidate: once the home screen has painted *from the cache*,
    // refresh the recent-projects list from GitHub in the background and
    // re-resolve the view only when it changed (an unchanged refresh is a no-op,
    // so there is no flicker). A failed refresh leaves the cached list in place.
    //
    // The guard ensures exactly one refresh per home visit: a cold-cache paint
    // is `from_cache: false` (it already fetched live, so we skip), and the
    // `reload` bump that swaps in a `Changed` list re-paints `from_cache: true`
    // but finds the guard set, so it does not fetch a second time.
    use_effect(move || {
        let from_cache = matches!(
            view.read().as_ref(),
            Some(View::Home {
                from_cache: true,
                ..
            })
        );
        if from_cache && !revalidated() {
            revalidated.set(true);
            spawn(async move {
                if let Ok(ProjectsRefresh::Changed(_)) = refresh_recent_projects().await {
                    reload += 1;
                }
            });
        }
    });

    // Background poll: while a board is open, re-resolve it on a fixed cadence so
    // the columns, counts, and "last updated" timestamp stay fresh without the
    // user clicking Refresh. The interval is the configurable `PollInterval`
    // (default ~60s); it only bumps `reload` when a board is showing, so the
    // home and paste-token screens are never disturbed.
    use_future(move || async move {
        let interval = PollInterval::default().as_duration();
        loop {
            // `tokio::time::sleep` is fine while v1 is a standalone desktop app
            // running on Dioxus's tokio runtime. It depends on tokio's time
            // driver, which is unavailable on `wasm32`, so if a web/server
            // presentation is added later this timer should move behind a
            // desktop-only `cfg` (or swap to a wasm-portable timer like
            // `futures-timer`) when we gate server/web/desktop.
            tokio::time::sleep(interval).await;
            // `peek` reads the latest view without subscribing, so this loop is
            // never restarted by its own re-resolves.
            if matches!(&*view.peek(), Some(View::Board { .. })) {
                reload += 1;
            }
        }
    });

    // Manual refresh: re-resolve the current view on demand (Refresh button).
    let on_refresh = move |_| {
        reload += 1;
    };

    let on_submit = move |raw: String| {
        spawn(async move {
            saving.set(true);
            let auth = AuthService::new(secure_store());
            match auth.save_token(&raw).await {
                Ok(()) => {
                    token_error.set(None);
                    reload += 1;
                }
                Err(error) => token_error.set(Some(error.to_string())),
            }
            saving.set(false);
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
    // Reset the revalidate guard so returning to Home refreshes the list again.
    let on_home = move |_| {
        nav.set(Nav::Home);
        revalidated.set(false);
        reload += 1;
    };

    // True only on a *cold* board load: the user navigated to a project whose
    // board is not on screen yet (opening it from home, or reopening the last
    // one on launch). A board *self-refresh* — the background poll, the Refresh
    // button, or the re-poll after assigning/confirming — leaves the populated
    // board as the current value, so this stays false and the board keeps
    // showing instead of flashing a spinner over it.
    let board_loading = matches!(*view.state().read(), UseResourceState::Pending)
        && match (nav(), &*view.read_unchecked()) {
            (Nav::Project(target), Some(View::Board { repo, .. })) => *repo != target,
            (Nav::Project(_), _) => true,
            _ => false,
        };

    // True while an *already-shown* board is silently re-resolving: the
    // background poll, the Refresh button, or the re-poll after assigning or
    // confirming. The board stays on screen (see `board_loading`), so this only
    // drives a small in-flight indicator on the Refresh button rather than
    // replacing any content.
    let refreshing = matches!(*view.state().read(), UseResourceState::Pending)
        && matches!(&*view.read_unchecked(), Some(View::Board { .. }))
        && !board_loading;

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        match (&*view.read_unchecked(), board_loading, nav()) {
            // Navigating to a board we do not have yet: opening a project from
            // the home screen or reopening one on launch. Show the board chrome
            // with a spinner so the navigation has immediate feedback. A board
            // that is merely self-refreshing keeps `board_loading` false and so
            // falls through to the populated `View::Board` arm below.
            (_, true, Nav::Project(repo)) => rsx! {
                BoardShell { repo: repo.to_string(), on_home,
                    div { class: "flex justify-center py-16",
                        Spinner { label: "Loading board…" }
                    }
                }
            },
            // Token present but no project open: show recent projects. Kept
            // visible while the list silently revalidates (stale-while-revalidate)
            // so the background refresh does not flash a spinner.
            (Some(View::Home { projects, .. }), ..) => rsx! {
                HomeScreen { projects: projects.clone(), on_open }
            },
            // No token yet, or a stored token was rejected: show the paste-token
            // screen. A fresh submit error takes precedence over a stale reject
            // reason carried by the view. `saving` drives the submit spinner while
            // a freshly-pasted token is validated.
            (Some(View::NeedToken { reason }), ..) => rsx! {
                TokenScreen {
                    error: token_error().or_else(|| reason.clone()),
                    saving: saving(),
                    on_submit,
                }
            },
            // Token present and the board loaded.
            (Some(View::Board { repo, board, loaded_at }), ..) => {
                // Claim the Slice on GitHub, then re-poll so the now-assigned
                // Slice derives Wip and leaves Ready. On failure the board is
                // left unchanged and the error is surfaced above it.
                let assign_repo = repo.clone();
                let on_assign = move |number: u64| {
                    let repo = assign_repo.clone();
                    spawn(async move {
                        match assign_self(&repo, number).await {
                            Ok(()) => {
                                assign_error.set(None);
                                reload += 1;
                            }
                            Err(error) => assign_error.set(Some(error.to_string())),
                        }
                    });
                };
                // Confirm a suggested classification: add its prd/slice label,
                // then re-poll so the now-labelled issue classifies tier-1 and
                // leaves "other open issues". On failure the issue is left
                // unchanged and the error is surfaced above the board.
                let confirm_repo = repo.clone();
                let on_confirm = move |(number, classification): (u64, IssueClassification)| {
                    let repo = confirm_repo.clone();
                    spawn(async move {
                        match confirm_classification(&repo, number, &classification).await {
                            Ok(()) => {
                                confirm_error.set(None);
                                reload += 1;
                            }
                            Err(error) => confirm_error.set(Some(error.to_string())),
                        }
                    });
                };
                let summary = BoardSummary::from_slices(&board.slices);
                rsx! {
                    BoardShell {
                        repo: repo.to_string(),
                        on_home,
                        on_refresh,
                        refreshing,
                        last_updated: loaded_at.clone(),
                        if let Some(message) = assign_error() {
                            ErrorBanner { message }
                        }
                        if let Some(message) = confirm_error() {
                            ErrorBanner { message }
                        }
                        BoardSummaryBar { summary }
                        Board { slices: board.slices.clone(), on_assign }
                        if !board.other.is_empty() {
                            OtherIssues { issues: board.other.clone(), on_confirm }
                        }
                    }
                }
            }
            (Some(View::Error(message)), ..) => rsx! {
                BoardShell { on_home, on_refresh,
                    ErrorBanner { message: message.clone() }
                }
            },
            (None, ..) => rsx! {
                LoadingScreen { label: "Loading…" }
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

    match state.classify_board().await {
        Ok(board) => View::Board {
            repo,
            board,
            loaded_at: now_hms(),
        },
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

/// The home screen's recent-projects view, with stale-while-revalidate caching:
/// a warm cache paints instantly (the background refresh in `App` revalidates
/// it), while a cold cache falls back to a blocking live fetch — shown with the
/// loading state — that also seeds the cache. A token GitHub rejects while
/// listing (`Unauthorized`/`Forbidden`) is discarded and routes back to the
/// paste-token screen, just like the board path; any other failure is transient.
async fn home_view<S: SecureStorePort>(auth: &AuthService<S>, token: &GitHubToken) -> View {
    // Warm cache: render immediately without waiting on GitHub. `from_cache`
    // tells the background effect this paint still needs revalidating.
    if let Ok(Some(projects)) = cached_projects().await {
        return View::Home {
            projects,
            from_cache: true,
        };
    }
    // Cold (or unreadable) cache: block on a live fetch that seeds the cache.
    // Either way the list is now live, so `from_cache` is false (no re-fetch).
    match refresh_projects(token).await {
        Ok(ProjectsRefresh::Changed(projects)) => View::Home {
            projects,
            from_cache: false,
        },
        // The cache was populated concurrently and already matches the live
        // list, so read it back. The refresh just confirmed it exists, so a
        // missing or unreadable cache here means something raced or failed:
        // surface it rather than render a misleadingly empty home.
        Ok(ProjectsRefresh::Unchanged) => match cached_projects().await {
            Ok(Some(projects)) => View::Home {
                projects,
                from_cache: false,
            },
            Ok(None) => View::Error("The cached projects vanished during refresh.".into()),
            Err(error) => View::Error(error.to_string()),
        },
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

/// The current local wall-clock time as `HH:MM:SS`, captured when a board
/// snapshot is loaded so it can be shown as the "last updated" timestamp.
fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// The board chrome (header + logo) wrapping either the columns or an error.
/// `repo` names the open project (shown beside the title) when there is one;
/// `on_home` returns to the project picker. When `on_refresh` is set a Refresh
/// button re-polls the board on demand, and `last_updated` shows when the
/// current snapshot was loaded. While `refreshing` is set that button shows an
/// inline spinner and is disabled, so an in-flight refresh has feedback without
/// disturbing the board content.
#[component]
fn BoardShell(
    children: Element,
    on_home: EventHandler<()>,
    #[props(default)] repo: Option<String>,
    #[props(default)] on_refresh: Option<EventHandler<()>>,
    #[props(default)] refreshing: bool,
    #[props(default)] last_updated: Option<String>,
) -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "flex items-center gap-2 mb-6",
                ZfirotLogo {}
                h1 { class: "text-2xl font-bold", "Zfirot" }
                if let Some(repo) = repo {
                    span { class: "text-base opacity-60", "/ {repo}" }
                }
                // Always available so an error view (which carries no `repo`)
                // still has a navigation escape hatch back to the project picker.
                button {
                    class: "btn btn-ghost btn-sm btn-square",
                    title: "Back to projects",
                    aria_label: "Back to projects",
                    onclick: move |_| on_home.call(()),
                    span { class: "icon-[lucide--undo-2] size-5" }
                }
                // Freshness controls, pushed to the right.
                div { class: "ml-auto flex items-center gap-3",
                    if let Some(updated) = last_updated {
                        span { class: "text-xs opacity-60", "Updated {updated}" }
                    }
                    if let Some(on_refresh) = on_refresh {
                        button {
                            class: "btn btn-ghost btn-sm btn-square",
                            title: "Refresh now",
                            aria_label: "Refresh now",
                            disabled: refreshing,
                            onclick: move |_| on_refresh.call(()),
                            if refreshing {
                                span { class: "loading loading-spinner size-5" }
                            } else {
                                span { class: "icon-[lucide--refresh-cw] size-5" }
                            }
                        }
                    }
                }
            }
            {children}
        }
    }
}

/// A summary strip of how many Slices sit in each board state, shown above the
/// columns so the project's status is legible at a glance.
#[component]
fn BoardSummaryBar(summary: BoardSummary) -> Element {
    rsx! {
        div { class: "flex items-center gap-2 mb-4",
            span { class: "badge badge-success badge-outline gap-1",
                "Ready"
                span { class: "font-semibold", "{summary.ready}" }
            }
            span { class: "badge badge-warning badge-outline gap-1",
                "WIP"
                span { class: "font-semibold", "{summary.wip}" }
            }
            span { class: "badge badge-error badge-outline gap-1",
                "Blocked"
                span { class: "font-semibold", "{summary.blocked}" }
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
fn Board(slices: Vec<Slice>, on_assign: EventHandler<u64>) -> Element {
    let lanes = group_into_lanes(slices);
    // The board-wide "highlighted issue", shared across lanes so a dependency
    // badge can highlight its referenced card in any column. `None` when nothing
    // is hovered.
    let mut highlighted = use_signal(|| Option::<u64>::None);
    rsx! {
        div { class: "flex flex-col gap-6",
            for lane in lanes {
                PrdLane {
                    key: "{lane.prd.as_ref().map(|prd| prd.number).unwrap_or(0)}",
                    prd: lane.prd,
                    slices: lane.slices,
                    on_assign: move |number| on_assign.call(number),
                    highlighted: highlighted(),
                    on_highlight: move |number| highlighted.set(number),
                }
            }
        }
    }
}

/// The "other open issues" bucket — shows suggested and unclassified issues
/// below the Kanban board.
///
/// Suggested issues (tier-2 classification) render with a
/// "looks like a PRD/Slice — confirm?" badge and a Confirm button that emits
/// `on_confirm` with the issue number and its classification; the board then
/// adds the `prd`/`slice` label and re-polls. Unclassified issues render
/// without any badge or action.
#[component]
fn OtherIssues(
    issues: Vec<OtherIssue>,
    on_confirm: EventHandler<(u64, IssueClassification)>,
) -> Element {
    let count = issues.len();
    rsx! {
        section { class: "mt-6",
            div { class: "collapse collapse-arrow bg-base-100 border border-base-300",
                input { r#type: "checkbox" }
                div { class: "collapse-title text-lg font-semibold flex items-center gap-2",
                    "Other open issues"
                    span { class: "badge badge-neutral", "{count}" }
                }
                div { class: "collapse-content",
                    div { class: "flex flex-col gap-2",
                        for issue in issues {
                            OtherIssueCard {
                                key: "{issue.number}",
                                issue: issue.clone(),
                                on_confirm: move |payload| on_confirm.call(payload),
                            }
                        }
                    }
                }
            }
        }
    }
}
