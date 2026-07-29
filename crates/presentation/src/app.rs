//! Root component: gates the board behind a stored Personal Access Token.
//!
//! On launch the stored token (OS secure store) is resolved. With a token the
//! real board loads; without one the paste-token screen is shown. Saving a valid
//! token persists it and loads the board. A stored token that GitHub rejects is
//! discarded and the user is routed back to the paste-token screen to enter a
//! new one, with the reason shown inline.

use application::{
    AuthService, BoardCacheUsage, BoardOpen, BoardRefresh, BoardSnapshot, ClassifiedBoard,
    OtherIssue, ProjectsRefresh, SecureStorePort,
};
use dioxus::prelude::*;
use domain::{
    group_into_lanes, AppError, AppErrorKind, BoardSummary, BoardViewMode, CredentialFailure,
    GitHubToken, IssueClassification, PollInterval, Project, ReconcileInterval, RepoRef, Slice,
    ThemePreference, Viewer,
};
use std::{future::Future, sync::Arc};

use crate::components::{
    AccountMenu, ErrorBanner, HomeScreen, LoadingScreen, OtherIssueCard, PrdLane, Spinner,
    TokenScreen,
};
use crate::state::{
    assign_self, cache_usage, cached_projects, clear_all_board_cache, clear_board_cache,
    confirm_classification, last_opened, open_and_track_project, open_board, open_project,
    reconcile_board, refresh_board, refresh_projects, refresh_recent_projects,
    remember_theme_preference, remember_view_mode, secure_store, sign_out, theme_preference,
    tracked_repos, untrack_repo, view_mode, viewer as fetch_viewer,
};
use tracing::warn;

/// Compiled Tailwind + daisyUI + Iconify stylesheet, bundled as an asset.
/// `dx serve` / `dx bundle` (Dioxus 0.7) auto-generate it from
/// `crates/presentation/tailwind.css`; for plain `cargo run`, `make css`
/// regenerates it. The generated file is untracked (see .gitignore).
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// What the root renders once the stored token has been resolved.
enum View {
    /// A token is stored but no project is open yet: pick from recent projects.
    /// `from_cache` is `true` only when this list came from an instant cached
    /// paint that still needs revalidating; a list produced by a live fetch
    /// (cold-cache fallback or a completed refresh) sets it `false` so the
    /// background effect does not fetch again. `tracked_repos` are shown in a
    /// separate section.
    Home {
        projects: Vec<Project>,
        tracked_repos: Vec<RepoRef>,
        from_cache: bool,
    },
    /// A token is stored and the board for `repo` loaded and was classified into
    /// confirmed Slices plus an "other open issues" bucket. `loaded_at` is the
    /// local wall-clock time this snapshot was fetched, shown as "last updated".
    Board {
        repo: RepoRef,
        board: ClassifiedBoard,
        loaded_at: String,
        snapshot: BoardSnapshot,
        from_cache: bool,
    },
    /// Show the paste-token screen. `reason` is `Some` when a stored token was
    /// rejected by GitHub (so the user knows why they are being asked again) and
    /// `None` on first launch when no token has ever been saved.
    NeedToken { reason: Option<String> },
    /// A stored token is valid but GitHub refused it a specific grant while
    /// loading (`CredentialFailure::UnderScoped`): the token is kept (never
    /// cleared) and the Change-token (Rotate) screen is shown reactively so the
    /// user can grant the missing permission without losing a perfectly usable
    /// credential. `missing_permission` names the grant when GitHub's error
    /// says so, for the highlighted item in the required-permissions list.
    NeedRotate {
        reason: String,
        missing_permission: Option<String>,
    },
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
    /// Show the board for a project the user just chose from the recent list.
    Project(RepoRef),
    /// Open a project summoned by name (the go-to action): load its board and,
    /// on success, track it. Resolved once via `open_and_track_project` so a
    /// go-to open fetches the board only once (no verify-then-reopen double
    /// fetch), and a failure surfaces like any other board load.
    GoTo(RepoRef),
}

/// How a feature action (assign self, confirm classification) failed — what
/// `ErrorBanner` renders for it. An `Invalid` credential failure never reaches
/// this type: it clears the token and navigates to the paste screen instead of
/// leaving any banner behind (see `feature_action_error`).
#[derive(Clone, PartialEq)]
enum FeatureActionError {
    /// Plain transient banner (network, rate limit, not found, a bug, …),
    /// unchanged from before this ticket.
    Plain(String),
    /// Actionable banner: the explanation plus a "Change token…" button that
    /// opens the Rotate flow, per #144.
    UnderScoped(String),
}

#[derive(Clone)]
struct ReconcileGate(Arc<tokio::sync::Mutex<()>>);

impl ReconcileGate {
    fn new() -> Self {
        Self(Arc::new(tokio::sync::Mutex::new(())))
    }

    // Best-effort path for automatic reconciles: skip when another reconcile
    // already holds the gate.
    async fn try_run<F, Fut>(&self, body: F)
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        if let Ok(_permit) = self.0.clone().try_lock_owned() {
            body().await;
        }
    }

    // Manual refresh path: always run, waiting for any in-flight reconcile.
    async fn run<F, Fut>(&self, body: F)
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ()>,
    {
        let _permit = self.0.clone().lock_owned().await;
        body().await;
    }
}

#[component]
pub fn App() -> Element {
    // Cleared on success, set to a client-safe message when a token is rejected.
    let mut token_error = use_signal(|| Option::<String>::None);
    // Set to a client-safe message when assigning self to a Slice fails, shown
    // above the board; cleared on a successful assignment. The delegate (Agent)
    // action reuses this same banner. `None` (an Invalid credential failure)
    // never lands here — that clears the token and navigates away instead.
    let mut assign_error = use_signal(|| Option::<FeatureActionError>::None);
    // Set to a client-safe message when confirming a suggested classification
    // fails, shown above the board; cleared on a successful confirm. Same
    // Invalid-never-lands-here rule as `assign_error`.
    let mut confirm_error = use_signal(|| Option::<FeatureActionError>::None);
    // Set to a client-safe message when signing out fails, shown above the
    // current screen (Home or Board); cleared on a successful sign-out.
    let mut sign_out_error = use_signal(|| Option::<String>::None);
    // Bumped after a successful save or project selection so the view re-resolves.
    let mut reload = use_signal(|| 0u32);
    // Where the user wants to be; starts at `Auto` so the last-opened project
    // reopens on launch, and switches to `Home`/`Project` as they navigate.
    let mut nav = use_signal(|| Nav::Auto);
    // Guards the stale-while-revalidate refresh so it runs once per visit to the
    // home screen, not on every `reload` bump. Reset when the user navigates
    // back to Home so returning revalidates again.
    let mut revalidated = use_signal(|| false);
    // Guards the board stale-while-revalidate refresh so a cached board paints
    // instantly and then revalidates once per open.
    let mut board_revalidated = use_signal(|| Option::<RepoRef>::None);
    // True while a freshly-pasted token is being validated and persisted. This
    // save is a mutation, not part of the read-only `view` resource, so it needs
    // its own in-flight flag to drive the spinner on the submit button.
    let mut saving = use_signal(|| false);
    // Retained board snapshot used by `BoardService::refresh` to decide
    // Changed/Unchanged during polls and manual refreshes.
    let mut board_snapshot = use_signal(|| Option::<BoardSnapshot>::None);
    // True while the fast/manual delta refresh is in flight; it clears before
    // the authoritative reconcile runs.
    let mut board_refreshing = use_signal(|| false);
    // Single-flight gate for full-load reconciles so background and manual
    // reconciles never overlap and stale results cannot overwrite newer state.
    let reconcile_gate = use_hook(ReconcileGate::new);
    // A board view fetched by a `Changed` refresh, handed to the next `view`
    // resolution so it repaints the fresh board without a second network fetch.
    let mut prefetched_board = use_signal(|| Option::<View>::None);
    // Global board-cache usage shown in the top bar.
    let mut cache_stats = use_signal(BoardCacheUsage::default);
    // The currently active theme for the toggle UI. When no preference is stored,
    // this is initialised from the OS `prefers-color-scheme`.
    let mut theme = use_signal(|| ThemePreference::Light);
    // Session board mode: false = columns (default), true = graph. Initialised
    // once from the persisted view mode on launch.
    let mut graph_view = use_signal(|| false);
    // Runs theme initialisation exactly once.
    let mut theme_initialized = use_signal(|| false);
    // Runs board view-mode initialisation exactly once.
    let mut view_mode_initialized = use_signal(|| false);
    // The signed-in Viewer (avatar, login), for the account menu. `None` while
    // loading or if the fetch failed — the menu falls back to a generic icon.
    let mut viewer = use_signal(|| Option::<Viewer>::None);
    // Guards the Viewer fetch so it runs once per app session, not on every
    // `reload` bump.
    let mut viewer_fetch_attempted = use_signal(|| false);
    // True while the Change-token (Rotate) screen is shown as a full-screen
    // overlay in front of whatever was on screen (Home or a board). Toggled by
    // the account menu's "Change token…" item; Cancel or a successful save
    // clears it, returning to that same underlying view untouched.
    let mut show_change_token = use_signal(|| false);
    // Cleared on a successful Change-token save, set to a client-safe message
    // when the pasted token is rejected. Independent from `token_error` (the
    // first-launch paste-token screen) so the two flows never share state.
    let mut change_token_error = use_signal(|| Option::<String>::None);
    // True while a freshly-pasted replacement token is being validated and
    // persisted, driving the Change-token screen's own spinner.
    let mut change_token_saving = use_signal(|| false);
    // Drives the reactive Change-token (Rotate) screen shown directly in place
    // of a failed board/project-list load when the stored token is merely
    // Under-scoped (`View::NeedRotate`) — kept separate from `change_token_*`
    // above (the account-menu-triggered overlay) since the two never show at
    // once and a successful submit here must retry the load instead of just
    // dismissing an overlay.
    let mut rotate_reactive_error = use_signal(|| Option::<String>::None);
    let mut rotate_reactive_saving = use_signal(|| false);

    let view = use_resource(move || async move {
        let _ = reload(); // subscribe so a save or selection re-resolves the view
                          // A `Changed` refresh stashed the freshly-fetched board here; repaint it
                          // directly instead of fetching the same board a second time.
        if prefetched_board.peek().is_some() {
            if let Some(prefetched) = prefetched_board.write().take() {
                return prefetched;
            }
        }
        resolve_view(nav()).await
    });

    // Keep the retained board snapshot in a signal while a board is open.
    use_effect(move || match view.read().as_ref() {
        Some(View::Board { snapshot, .. }) => board_snapshot.set(Some(snapshot.clone())),
        _ => board_snapshot.set(None),
    });

    // Keep the top-bar cache indicator in sync with local cache writes/clears.
    use_effect(move || {
        let _ = reload();
        spawn(async move {
            if let Ok(usage) = cache_usage().await {
                cache_stats.set(usage);
            }
        });
    });

    // Resolve persisted theme once on launch. With no stored preference, leave
    // `data-theme` unset so daisyUI follows the OS `prefers-color-scheme`.
    use_effect(move || {
        if theme_initialized() {
            return;
        }
        theme_initialized.set(true);
        spawn(async move {
            match theme_preference().await {
                Ok(Some(stored)) => {
                    apply_data_theme(stored);
                    theme.set(stored);
                }
                Ok(None) | Err(_) => {
                    let system = resolve_system_theme().await;
                    apply_data_theme(system);
                    theme.set(system);
                }
            }
        });
    });

    // Resolve the persisted board view mode once on launch. With no stored
    // preference the board opens in columns view (the default); a stored Graph
    // mode reopens the graph.
    use_effect(move || {
        if view_mode_initialized() {
            return;
        }
        view_mode_initialized.set(true);
        spawn(async move {
            if let Ok(Some(BoardViewMode::Graph)) = view_mode().await {
                graph_view.set(true);
            }
        });
    });

    // Fetch the signed-in Viewer once a token is present (Home or Board showing),
    // so the account menu can show a real avatar + login. A fetch failure simply
    // leaves `viewer` at `None`, which the menu reads as "show a generic icon" —
    // never blocking on it.
    use_effect(move || {
        let has_token = matches!(
            view.read().as_ref(),
            Some(View::Home { .. }) | Some(View::Board { .. })
        );
        if has_token && !viewer_fetch_attempted() {
            viewer_fetch_attempted.set(true);
            spawn(async move {
                if let Ok(fetched) = fetch_viewer().await {
                    viewer.set(Some(fetched));
                }
            });
        }
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

    // Stale-while-revalidate for project boards: if a board was painted from the
    // local cache, refresh it once in the background — a fast delta first, then
    // a silent authoritative reconcile (same two-phase pattern as the manual
    // Refresh button) — and repaint only if either phase finds a real change.
    // The reconcile matters here specifically: GitHub does not bump an issue's
    // own `updatedAt` when a PR is opened with a closing reference to it, so a
    // just-opened linked PR can be invisible to the delta's `since` filter for
    // as long as the board stays open, until the reconcile re-derives it from a
    // full load.
    let reconcile_gate_on_open = reconcile_gate.clone();
    use_effect(move || {
        let cached_board = match view.read().as_ref() {
            Some(View::Board {
                repo,
                snapshot,
                from_cache: true,
                ..
            }) => Some((repo.clone(), snapshot.clone())),
            _ => None,
        };
        if let Some((repo, snapshot)) = cached_board {
            if board_revalidated().as_ref() == Some(&repo) {
                return;
            }
            board_revalidated.set(Some(repo.clone()));
            let gate = reconcile_gate_on_open.clone();
            spawn(async move {
                if let Some(base) = delta_refresh(
                    repo.clone(),
                    snapshot,
                    board_snapshot,
                    prefetched_board,
                    reload,
                )
                .await
                {
                    // Best-effort: automatic background checks skip if another
                    // reconcile is already in flight.
                    gate.clone()
                        .try_run(|| {
                            authoritative_reconcile(
                                repo,
                                base,
                                board_snapshot,
                                prefetched_board,
                                reload,
                            )
                        })
                        .await;
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
            // Extract the open repo in a tight scope so the `view` borrow is
            // dropped before the `.await` below. Holding a `peek()` guard across
            // the await lets a concurrent re-resolve of the resource (e.g. a
            // manual refresh bumping `reload`) double-borrow it and panic with
            // `AlreadyBorrowed`. `peek` also avoids subscribing this loop.
            let open_repo = match &*view.peek() {
                Some(View::Board { repo, .. }) => Some(repo.clone()),
                _ => None,
            };
            if let Some(repo) = open_repo {
                if let Some(snapshot) = board_snapshot() {
                    match refresh_board(&repo, &snapshot).await {
                        Ok(BoardRefresh::Changed(loaded)) => {
                            // Stash the already-fetched board and repaint from it, so
                            // a change costs one fetch, not two (refresh + reload).
                            board_snapshot.set(Some(loaded.snapshot.clone()));
                            prefetched_board.set(Some(View::Board {
                                repo,
                                board: loaded.board,
                                loaded_at: now_hms(),
                                snapshot: loaded.snapshot,
                                from_cache: false,
                            }));
                            reload += 1;
                        }
                        // Facts unchanged: don't repaint, but adopt the snapshot so
                        // its advanced `fetched_at` moves the next delta window on.
                        Ok(BoardRefresh::Unchanged(snapshot)) => {
                            board_snapshot.set(Some(snapshot));
                        }
                        Err(error) => {
                            warn!(repo = %repo, error = ?error, "background reconcile failed");
                        }
                    }
                }
            }
        }
    });

    // Slow background reconcile: full-load occasionally to heal any drift that
    // delta refreshes cannot observe (for example, hard-deleted or transferred
    // issues). Silent and non-blocking: repaint only on a real diff.
    let reconcile_gate_background = reconcile_gate.clone();
    use_future(move || {
        let gate = reconcile_gate_background.clone();
        async move {
            let interval = ReconcileInterval::default().as_duration();
            loop {
                tokio::time::sleep(interval).await;
                // Drop the `view` borrow before awaiting (see the poll loop above):
                // a held `peek()` guard across the await races a resource re-resolve
                // and panics with `AlreadyBorrowed`.
                let open_repo = match &*view.peek() {
                    Some(View::Board { repo, .. }) => Some(repo.clone()),
                    _ => None,
                };
                if let Some(repo) = open_repo {
                    if let Some(snapshot) = board_snapshot() {
                        // Best-effort: periodic background checks skip if another
                        // reconcile is already in flight.
                        gate.clone()
                            .try_run(|| {
                                authoritative_reconcile(
                                    repo,
                                    snapshot,
                                    board_snapshot,
                                    prefetched_board,
                                    reload,
                                )
                            })
                            .await;
                    }
                }
            }
        }
    });

    // Manual refresh: a fast delta for instant feedback, then a silent,
    // authoritative reconcile to heal anything the delta cannot see (e.g. an
    // issue closed by a merged PR, whose close event may fall outside the delta
    // `since` window). The delta paints first and clears the spinner; the
    // reconcile runs full-load and repaints only on a real difference.
    //
    // The reconcile call below is deliberately unconditional (unlike the two
    // automatic reconcile sources): a user clicking Update must always get a
    // full reconciliation, never a silent no-op because a background reconcile
    // happened to be running at that moment — losing that guarantee is exactly
    // what let a Slice's linked-PR badge go stale despite the user clicking
    // Update (#149, hardening the guarantee #126 first established).
    let reconcile_gate_manual = reconcile_gate.clone();
    let on_refresh = move |_| {
        if board_refreshing() {
            return;
        }
        let open_repo = match view.read_unchecked().as_ref() {
            Some(View::Board { repo, .. }) => Some(repo.clone()),
            _ => None,
        };
        if let Some(repo) = open_repo {
            if let Some(snapshot) = board_snapshot() {
                let gate = reconcile_gate_manual.clone();
                // Set the in-flight guard synchronously (before the spawn) so
                // rapid clicks cannot start a second refresh between the check
                // above and the flag being set inside the task.
                board_refreshing.set(true);
                spawn(async move {
                    let base = delta_refresh(
                        repo.clone(),
                        snapshot,
                        board_snapshot,
                        prefetched_board,
                        reload,
                    )
                    .await;
                    // Spinner clears after the fast delta paint.
                    board_refreshing.set(false);

                    if let Some(base) = base {
                        gate.clone()
                            .run(|| {
                                authoritative_reconcile(
                                    repo,
                                    base,
                                    board_snapshot,
                                    prefetched_board,
                                    reload,
                                )
                            })
                            .await;
                    }
                });
                return;
            }
        }
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

    // Open the Change-token (Rotate) overlay from the account menu, on top of
    // whatever is currently shown (Home or a board).
    let on_open_change_token = move |_| {
        change_token_error.set(None);
        show_change_token.set(true);
    };

    // Cancel: dismiss the overlay without touching the stored token, returning
    // to exactly the view that was showing underneath.
    let on_change_token_cancel = move |_| {
        change_token_error.set(None);
        show_change_token.set(false);
    };

    // Save the replacement token via the existing `AuthService::save_token`
    // (an atomic overwrite — no prior delete — so the old token keeps working
    // until this succeeds). On success, dismiss the overlay: the underlying
    // Home/board view and all local state (tracked repos, caches, theme, view
    // mode) are untouched, since the same Viewer is still signed in.
    let on_change_token_submit = move |raw: String| {
        spawn(async move {
            change_token_saving.set(true);
            let auth = AuthService::new(secure_store());
            match auth.save_token(&raw).await {
                Ok(()) => {
                    change_token_error.set(None);
                    show_change_token.set(false);
                }
                Err(error) => change_token_error.set(Some(error.to_string())),
            }
            change_token_saving.set(false);
        });
    };

    // Submit for the reactive `View::NeedRotate` screen: unlike
    // `on_change_token_submit` above (which just dismisses an overlay over an
    // already-showing view), there is no underlying view here — the load that
    // produced `NeedRotate` failed — so a successful save must retry it instead.
    let on_rotate_reactive_submit = move |raw: String| {
        // Set before `spawn`, not inside it: a fast repeat click must see
        // `saving` already true, not slip through a check-then-set gap.
        rotate_reactive_saving.set(true);
        spawn(async move {
            let auth = AuthService::new(secure_store());
            match auth.save_token(&raw).await {
                Ok(()) => {
                    rotate_reactive_error.set(None);
                    prefetched_board.set(None);
                    reload += 1;
                }
                Err(error) => rotate_reactive_error.set(Some(error.to_string())),
            }
            rotate_reactive_saving.set(false);
        });
    };

    let on_open_discovered = use_callback(move |repo: RepoRef| {
        spawn(async move {
            // Persist the choice (best-effort) and navigate to its board.
            let _ = open_project(&repo).await;
            prefetched_board.set(None);
            board_revalidated.set(None);
            nav.set(Nav::Project(repo));
            reload += 1;
        });
    });

    let on_open_goto = use_callback(move |repo: RepoRef| {
        // The go-to open (load the board, track on success) happens in
        // `resolve_view` for `Nav::GoTo`, so this only routes there. Doing the
        // work there means the board is fetched once, and a failure (e.g. a 404)
        // surfaces through the normal view resolution rather than being dropped.
        nav.set(Nav::GoTo(repo));
        prefetched_board.set(None);
        board_revalidated.set(None);
        reload += 1;
    });

    let on_untrack = use_callback(move |repo: RepoRef| {
        spawn(async move {
            // Drop the repo from the tracked set, then re-resolve so the Tracked
            // card disappears. Best-effort: a store write failure simply leaves
            // the card in place rather than surfacing an error here.
            let _ = untrack_repo(&repo).await;
            reload += 1;
        });
    });

    let on_clear_cache_all = move |_| {
        spawn(async move {
            let _ = clear_all_board_cache().await;
            if let Ok(usage) = cache_usage().await {
                cache_stats.set(usage);
            }
        });
    };

    let on_clear_cache_repo = use_callback(move |repo: RepoRef| {
        spawn(async move {
            let _ = clear_board_cache(&repo).await;
            if let Ok(usage) = cache_usage().await {
                cache_stats.set(usage);
            }
        });
    });

    // Stable board-action handlers. Defined once (not re-minted inside the
    // `View::Board` render arm) so a card's event never invokes a dropped
    // generational box (`ValueDroppedError`) after the board re-renders — the
    // same rule the `Board` component documents for `on_highlight`. Each reads
    // the currently open repo from `view` at call time (borrow dropped before
    // the await).
    let on_assign = use_callback(move |number: u64| {
        let open_repo = match view.read_unchecked().as_ref() {
            Some(View::Board { repo, .. }) => Some(repo.clone()),
            _ => None,
        };
        if let Some(repo) = open_repo {
            spawn(async move {
                match assign_self(&repo, number).await {
                    Ok(()) => {
                        assign_error.set(None);
                        reload += 1;
                    }
                    Err(error) => assign_error
                        .set(feature_action_error(error, token_error, nav, reload).await),
                }
            });
        }
    });

    let on_confirm = use_callback(
        move |(number, classification): (u64, IssueClassification)| {
            let open_repo = match view.read_unchecked().as_ref() {
                Some(View::Board { repo, .. }) => Some(repo.clone()),
                _ => None,
            };
            if let Some(repo) = open_repo {
                spawn(async move {
                    match confirm_classification(&repo, number, &classification).await {
                        Ok(()) => {
                            confirm_error.set(None);
                            reload += 1;
                        }
                        Err(error) => confirm_error
                            .set(feature_action_error(error, token_error, nav, reload).await),
                    }
                });
            }
        },
    );

    // Sign out: remove the token and account-scoped local state, then
    // re-resolve the view — `resolve_view` finds no token and routes to the
    // paste-token screen. Session-only signals (the fetched Viewer, the
    // retained board snapshot, revalidation guards) are reset so a fresh
    // sign-in starts clean rather than reusing the prior account's state.
    let on_sign_out = move |_| {
        spawn(async move {
            match sign_out().await {
                Ok(()) => {
                    sign_out_error.set(None);
                    viewer.set(None);
                    viewer_fetch_attempted.set(false);
                    board_snapshot.set(None);
                    prefetched_board.set(None);
                    board_revalidated.set(None);
                    revalidated.set(false);
                    nav.set(Nav::Auto);
                    reload += 1;
                }
                Err(error) => sign_out_error.set(Some(error.to_string())),
            }
        });
    };

    // Back to the project picker. Persistence is untouched, so the next launch
    // still reopens the last project; this only changes the current session.
    // Reset the revalidate guard so returning to Home refreshes the list again.
    let on_home = move |_| {
        nav.set(Nav::Home);
        prefetched_board.set(None);
        board_revalidated.set(None);
        revalidated.set(false);
        reload += 1;
    };

    // Toggle and persist the selected app theme.
    let on_toggle_theme = move |_| {
        spawn(async move {
            let next = match theme() {
                ThemePreference::Light => ThemePreference::Dark,
                ThemePreference::Dark => ThemePreference::Light,
            };
            apply_data_theme(next);
            theme.set(next);
            let _ = remember_theme_preference(next).await;
        });
    };

    // True only on a *cold* board load: the user navigated to a project whose
    // board is not on screen yet (opening it from home, or reopening the last
    // one on launch). A board *self-refresh* — the background poll, the Refresh
    // button, or the re-poll after assigning/confirming — leaves the populated
    // board as the current value, so this stays false and the board keeps
    // showing instead of flashing a spinner over it.
    let board_loading = matches!(*view.state().read(), UseResourceState::Pending)
        && match (nav(), &*view.read_unchecked()) {
            (Nav::Project(target) | Nav::GoTo(target), Some(View::Board { repo, .. })) => {
                *repo != target
            }
            (Nav::Project(_) | Nav::GoTo(_), _) => true,
            _ => false,
        };

    // True while an *already-shown* board is silently re-resolving: the
    // background poll, the Refresh button, or the re-poll after assigning or
    // confirming. The board stays on screen (see `board_loading`), so this only
    // drives a small in-flight indicator on the Refresh button rather than
    // replacing any content.
    let refreshing = (matches!(*view.state().read(), UseResourceState::Pending)
        && matches!(&*view.read_unchecked(), Some(View::Board { .. }))
        && !board_loading)
        || board_refreshing();

    rsx! {
        document::Title { "Zfirot" }
        document::Stylesheet { href: TAILWIND_CSS }

        if show_change_token() {
            TokenScreen {
                title: "Change your Personal Access Token".to_string(),
                description: "Replace the token Zfirot uses to talk to GitHub — for example when a new feature needs a permission your current token doesn't grant yet. Your current token keeps working until the new one is saved, so cancelling leaves everything exactly as it was.".to_string(),
                error: change_token_error(),
                saving: change_token_saving(),
                on_submit: on_change_token_submit,
                on_cancel: on_change_token_cancel,
            }
        } else {
            match (&*view.read_unchecked(), board_loading, nav()) {
            // Navigating to a board we do not have yet: opening a project from
            // the home screen or reopening one on launch. Show the board chrome
            // with a spinner so the navigation has immediate feedback. A board
            // that is merely self-refreshing keeps `board_loading` false and so
            // falls through to the populated `View::Board` arm below.
            (_, true, Nav::Project(repo) | Nav::GoTo(repo)) => rsx! {
                BoardShell {
                    repo: repo.to_string(),
                    on_home,
                    theme: theme(),
                    on_toggle_theme,
                    cache_usage: cache_stats(),
                    on_clear_cache_all,
                    on_clear_cache_repo,
                    viewer: viewer(),
                    on_change_token: on_open_change_token,
                    on_sign_out,
                    div { class: "flex justify-center py-16",
                        Spinner { label: "Loading board…" }
                    }
                }
            },
            // Token present but no project open: show recent projects. Kept
            // visible while the list silently revalidates (stale-while-revalidate)
            // so the background refresh does not flash a spinner.
            (Some(View::Home { projects, tracked_repos, .. }), ..) => rsx! {
                HomeScreen {
                    projects: projects.clone(),
                    tracked_repos: tracked_repos.clone(),
                    on_open_discovered,
                    on_open_goto,
                    on_untrack,
                    viewer: viewer(),
                    on_change_token: on_open_change_token,
                    on_sign_out,
                }
                if let Some(message) = sign_out_error() {
                    ErrorBanner { message }
                }
            },
            (Some(View::NeedToken { reason }), ..) => rsx! {
                TokenScreen {
                    error: token_error().or_else(|| reason.clone()),
                    saving: saving(),
                    on_submit,
                }
            },
            // A stored token is valid but is missing a specific GitHub grant:
            // the token is never cleared here (only `NeedToken`/Invalid clears
            // it), so this reuses the same Change-token copy and permissions
            // list as the account-menu-triggered overlay, but with no Cancel —
            // there is nowhere to return to since the load that produced this
            // view failed. A successful submit retries the load.
            (Some(View::NeedRotate { reason, missing_permission }), ..) => rsx! {
                TokenScreen {
                    title: "Update your Personal Access Token".to_string(),
                    description: format!("{reason} — grant the missing permission below, then paste the updated token here. Your current token keeps working until the new one is saved."),
                    error: rotate_reactive_error(),
                    saving: rotate_reactive_saving(),
                    on_submit: on_rotate_reactive_submit,
                    highlighted_permission: missing_permission.clone(),
                }
            },
            // Token present and the board loaded.
            (Some(View::Board {
                repo,
                board,
                loaded_at,
                ..
            }), ..) => {
                let summary = BoardSummary::from_slices(&board.slices);
                let on_toggle_graph = move |_| {
                    let next = !graph_view();
                    graph_view.set(next);
                    let mode = if next {
                        BoardViewMode::Graph
                    } else {
                        BoardViewMode::Columns
                    };
                    spawn(async move {
                        let _ = remember_view_mode(mode).await;
                    });
                };
                rsx! {
                    BoardShell {
                        repo: repo.to_string(),
                        on_home,
                        theme: theme(),
                        on_toggle_theme,
                        graph_view: graph_view(),
                        on_toggle_graph,
                        on_refresh,
                        refreshing,
                        last_updated: loaded_at.clone(),
                        cache_usage: cache_stats(),
                        on_clear_cache_all,
                        on_clear_cache_repo,
                        viewer: viewer(),
                        on_change_token: on_open_change_token,
                        on_sign_out,
                        if let Some(error) = assign_error() {
                            FeatureActionErrorBanner { error, on_change_token: on_open_change_token }
                        }
                        if let Some(error) = confirm_error() {
                            FeatureActionErrorBanner { error, on_change_token: on_open_change_token }
                        }
                        if let Some(message) = sign_out_error() {
                            ErrorBanner { message }
                        }
                        BoardSummaryBar { summary }
                        Board {
                            slices: board.slices.clone(),
                            graph_view: graph_view(),
                            on_assign,
                        }
                        if !board.other.is_empty() {
                            OtherIssues { issues: board.other.clone(), on_confirm }
                        }
                    }
                }
            }
            (Some(View::Error(message)), ..) => rsx! {
                BoardShell {
                    on_home,
                    on_refresh,
                    theme: theme(),
                    on_toggle_theme,
                    cache_usage: cache_stats(),
                    on_clear_cache_all,
                    on_clear_cache_repo,
                    viewer: viewer(),
                    on_change_token: on_open_change_token,
                    on_sign_out,
                    ErrorBanner { message: message.clone() }
                }
            },
            (None, ..) => rsx! {
                LoadingScreen { label: "Loading…" }
            },
            }
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
    // shows the picker too. `GoTo` is resolved here so the go-to open fetches the
    // board exactly once and tracks on success.
    let repo = match nav {
        Nav::Home => return home_view(&auth, &token).await,
        Nav::Project(repo) => repo,
        Nav::GoTo(repo) => {
            // Go-to open: load and classify the board (tracking on success); a
            // failed load surfaces here rather than being silently dropped.
            return match open_and_track_project(&token, &repo).await {
                Ok(loaded) => View::Board {
                    repo,
                    board: loaded.board,
                    loaded_at: now_hms(),
                    snapshot: loaded.snapshot,
                    from_cache: false,
                },
                Err(error) => credential_failure_view(&auth, error).await,
            };
        }
        Nav::Auto => match last_opened().await {
            Ok(Some(repo)) => repo,
            Ok(None) => return home_view(&auth, &token).await,
            Err(error) => return View::Error(error.to_string()),
        },
    };

    match open_board(&repo).await {
        Ok(BoardOpen::Cached(loaded)) => View::Board {
            repo,
            board: loaded.board,
            loaded_at: now_hms(),
            snapshot: loaded.snapshot,
            from_cache: true,
        },
        Ok(BoardOpen::Cold(loaded)) => View::Board {
            repo,
            board: loaded.board,
            loaded_at: now_hms(),
            snapshot: loaded.snapshot,
            from_cache: false,
        },
        Err(error) => credential_failure_view(&auth, error).await,
    }
}

/// The home screen's recent-projects view, with stale-while-revalidate caching:
/// a warm cache paints instantly (the background refresh in `App` revalidates
/// it), while a cold cache falls back to a blocking live fetch — shown with the
/// loading state — that also seeds the cache. A token GitHub rejects while
/// listing (`Unauthorized`/`Forbidden`) is discarded and routes back to the
/// paste-token screen, just like the board path; any other failure is transient.
async fn home_view<S: SecureStorePort>(auth: &AuthService<S>, token: &GitHubToken) -> View {
    // Get tracked repos (best-effort; a read failure returns an empty list).
    let tracked = tracked_repos().await.unwrap_or_default();

    // Warm cache: render immediately without waiting on GitHub. `from_cache`
    // tells the background effect this paint still needs revalidating.
    if let Ok(Some(projects)) = cached_projects().await {
        return View::Home {
            projects,
            tracked_repos: tracked,
            from_cache: true,
        };
    }
    // Cold (or unreadable) cache: block on a live fetch that seeds the cache.
    // Either way the list is now live, so `from_cache` is false (no re-fetch).
    match refresh_projects(token).await {
        Ok(ProjectsRefresh::Changed(projects)) => View::Home {
            projects,
            tracked_repos: tracked,
            from_cache: false,
        },
        // The cache was populated concurrently and already matches the live
        // list, so read it back. The refresh just confirmed it exists, so a
        // missing or unreadable cache here means something raced or failed:
        // surface it rather than render a misleadingly empty home.
        Ok(ProjectsRefresh::Unchanged) => match cached_projects().await {
            Ok(Some(projects)) => View::Home {
                projects,
                tracked_repos: tracked,
                from_cache: false,
            },
            Ok(None) => View::Error("The cached projects vanished during refresh.".into()),
            Err(error) => View::Error(error.to_string()),
        },
        Err(error) => credential_failure_view(auth, error).await,
    }
}

fn apply_data_theme(theme: ThemePreference) {
    let script = format!(
        "document.documentElement.setAttribute('data-theme', '{}');",
        theme.as_data_theme()
    );
    document::eval(&script);
}

async fn resolve_system_theme() -> ThemePreference {
    let prefers_dark = document::eval(
        "return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;",
    )
    .join::<bool>()
    .await
    .unwrap_or(false);
    if prefers_dark {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    }
}

/// Map a GitHub-call failure to the view it should route to, per
/// [`CredentialFailure`]. `Invalid` clears the stored token (best-effort —
/// routing back is what matters) and asks for a fresh one, exactly as before.
/// `UnderScoped` **keeps the token** and opens the Change-token (Rotate)
/// screen instead: the credential is still perfectly usable, it is just
/// missing a grant, so wiping it would strand the user for no reason (the
/// original bug this ticket fixes). Anything else is a transient failure the
/// caller cannot act on by touching the token.
async fn credential_failure_view<S: SecureStorePort>(
    auth: &AuthService<S>,
    error: AppError,
) -> View {
    match CredentialFailure::classify(&error) {
        CredentialFailure::Invalid => {
            let _ = auth.clear_token().await;
            View::NeedToken {
                reason: Some(error.to_string()),
            }
        }
        CredentialFailure::UnderScoped { missing_permission } => View::NeedRotate {
            reason: error.to_string(),
            missing_permission,
        },
        CredentialFailure::None => View::Error(error.to_string()),
    }
}

/// Classify a feature-action failure (assign self, confirm classification)
/// into the banner it should leave, applying the same ADR 0005
/// credential-failure routing as [`credential_failure_view`] but for an action
/// that started from an already-open board rather than a page load. `Invalid`
/// clears the token and falls back to the paste screen — via `token_error` so
/// the reason survives the navigation, `Nav::Auto` + `reload` mirroring
/// `on_sign_out` — leaving no banner at all (`None`). `UnderScoped` and
/// anything else stay on the Board view with a banner: `UnderScoped` an
/// actionable one (the caller wires its "Change token…" button to open
/// Rotate), anything else a plain one, unchanged from before #144.
async fn feature_action_error(
    error: AppError,
    mut token_error: Signal<Option<String>>,
    mut nav: Signal<Nav>,
    mut reload: Signal<u32>,
) -> Option<FeatureActionError> {
    match CredentialFailure::classify(&error) {
        CredentialFailure::Invalid => {
            let auth = AuthService::new(secure_store());
            let _ = auth.clear_token().await;
            token_error.set(Some(error.to_string()));
            nav.set(Nav::Auto);
            reload += 1;
            None
        }
        CredentialFailure::UnderScoped { .. } => {
            Some(FeatureActionError::UnderScoped(error.to_string()))
        }
        CredentialFailure::None => Some(FeatureActionError::Plain(error.to_string())),
    }
}

/// The current local wall-clock time as `HH:MM:SS`, captured when a board
/// snapshot is loaded so it can be shown as the "last updated" timestamp.
fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// The fast half of a two-phase board refresh: a delta fetch that repaints
/// immediately on a real change (`prefetched_board` + `reload`) and otherwise
/// just advances the retained snapshot's `fetched_at` so the next delta window
/// moves forward. Returns the resulting snapshot on success (`Changed` or
/// `Unchanged`), for the caller to feed into [`authoritative_reconcile`];
/// `None` on a fetch failure, so the caller skips reconciling from a result
/// that may not reflect reality. Shared by the auto-revalidate-on-cache-open
/// effect and the manual Refresh button, so both apply the same rule.
async fn delta_refresh(
    repo: RepoRef,
    snapshot: BoardSnapshot,
    mut board_snapshot: Signal<Option<BoardSnapshot>>,
    mut prefetched_board: Signal<Option<View>>,
    mut reload: Signal<u32>,
) -> Option<BoardSnapshot> {
    match refresh_board(&repo, &snapshot).await {
        Ok(BoardRefresh::Changed(loaded)) => {
            board_snapshot.set(Some(loaded.snapshot.clone()));
            prefetched_board.set(Some(View::Board {
                repo,
                board: loaded.board,
                loaded_at: now_hms(),
                snapshot: loaded.snapshot.clone(),
                from_cache: false,
            }));
            reload += 1;
            Some(loaded.snapshot)
        }
        // Facts unchanged: don't repaint, but adopt the snapshot so its
        // advanced `fetched_at` moves the next delta window on.
        Ok(BoardRefresh::Unchanged(snapshot)) => {
            board_snapshot.set(Some(snapshot.clone()));
            Some(snapshot)
        }
        Err(_) => None,
    }
}

/// The slow, authoritative half of a two-phase board refresh: a silent full
/// reconcile that heals drift the delta cannot observe. This is not only for
/// hard-deleted/transferred issues (the original motivation for the slow
/// background reconcile loop): GitHub does not bump an issue's own
/// `updatedAt` when a PR is opened with a closing reference to it, so a
/// Slice's brand-new linked PR can be invisible to every `load_issues_since`
/// delta until a full load re-derives `closedByPullRequestsReferences` from
/// scratch.
///
/// Repaints (`prefetched_board` + `reload`) only on a real difference. Callers
/// serialize entry through [`ReconcileGate`] so concurrent reconciles cannot
/// race to overwrite state.
async fn authoritative_reconcile(
    repo: RepoRef,
    base: BoardSnapshot,
    mut board_snapshot: Signal<Option<BoardSnapshot>>,
    mut prefetched_board: Signal<Option<View>>,
    mut reload: Signal<u32>,
) {
    if let Ok(BoardRefresh::Changed(loaded)) = reconcile_board(&repo, &base).await {
        board_snapshot.set(Some(loaded.snapshot.clone()));
        prefetched_board.set(Some(View::Board {
            repo,
            board: loaded.board,
            loaded_at: now_hms(),
            snapshot: loaded.snapshot,
            from_cache: false,
        }));
        reload += 1;
    }
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
    theme: ThemePreference,
    on_toggle_theme: EventHandler<()>,
    #[props(default)] repo: Option<String>,
    #[props(default)] on_refresh: Option<EventHandler<()>>,
    #[props(default)] refreshing: bool,
    #[props(default)] last_updated: Option<String>,
    #[props(default)] cache_usage: BoardCacheUsage,
    #[props(default)] on_clear_cache_all: Option<EventHandler<()>>,
    #[props(default)] on_clear_cache_repo: Option<EventHandler<RepoRef>>,
    #[props(default)] graph_view: bool,
    #[props(default)] on_toggle_graph: Option<EventHandler<()>>,
    #[props(default)] viewer: Option<Viewer>,
    on_change_token: EventHandler<()>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let total_cache = format_bytes(cache_usage.total_bytes);
    rsx! {
        div { class: "min-h-screen bg-base-100 p-6",
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
                    div { class: "flex items-center gap-1",
                        div { class: "dropdown dropdown-end",
                            div {
                                class: "btn btn-ghost btn-sm gap-2",
                                tabindex: "0",
                                role: "button",
                                title: "Board cache usage",
                                span { class: "icon-[lucide--database] size-4" }
                                span { class: "text-xs font-medium", "{total_cache}" }
                            }
                            ul {
                                tabindex: "0",
                                class: "dropdown-content menu p-2 shadow bg-base-200 rounded-box w-80 mt-1 z-10",
                                li { class: "menu-title text-xs", span { "Cache usage" } }
                                if cache_usage.projects.is_empty() {
                                    li { span { class: "text-xs opacity-70", "No cached projects" } }
                                } else {
                                    for project in cache_usage.projects.iter().cloned() {
                                        li {
                                            div { class: "flex items-center gap-2 justify-between",
                                                span {
                                                    class: "text-xs truncate",
                                                    title: "{project.repo}",
                                                    "{project.repo}"
                                                }
                                                div { class: "flex items-center gap-2 shrink-0",
                                                    span { class: "badge badge-ghost badge-sm", "{format_bytes(project.bytes)}" }
                                                    if let Some(on_clear_cache_repo) = on_clear_cache_repo {
                                                        button {
                                                            class: "btn btn-ghost btn-xs btn-square",
                                                            title: "Clear project cache",
                                                            aria_label: "Clear project cache",
                                                            onclick: {
                                                                let clear_repo = project.repo.clone();
                                                                move |_| on_clear_cache_repo.call(clear_repo.clone())
                                                            },
                                                            span { class: "icon-[lucide--eraser] size-4" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(on_clear_cache_all) = on_clear_cache_all {
                            button {
                                class: "btn btn-ghost btn-sm btn-square",
                                title: "Clear all cache",
                                aria_label: "Clear all cache",
                                onclick: move |_| on_clear_cache_all.call(()),
                                span { class: "icon-[lucide--eraser] size-5" }
                            }
                        }
                    }
                    button {
                        class: "btn btn-ghost btn-sm btn-square",
                        title: "Toggle theme",
                        aria_label: "Toggle theme",
                        onclick: move |_| on_toggle_theme.call(()),
                        if theme == ThemePreference::Dark {
                            span { class: "icon-[lucide--moon] size-5" }
                        } else {
                            span { class: "icon-[lucide--sun] size-5" }
                        }
                    }
                    if let Some(on_toggle_graph) = on_toggle_graph {
                        button {
                            class: "btn btn-ghost btn-sm btn-square",
                            title: if graph_view { "Switch to columns view" } else { "Switch to graph view" },
                            aria_label: if graph_view { "Switch to columns view" } else { "Switch to graph view" },
                            onclick: move |_| on_toggle_graph.call(()),
                            if graph_view {
                                span { class: "icon-[lucide--layout-dashboard] size-5" }
                            } else {
                                span { class: "icon-[lucide--workflow] size-5" }
                            }
                        }
                    }
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
                    AccountMenu { viewer, on_change_token, on_sign_out }
                }
            }
            {children}
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else if bytes as f64 >= KB {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{bytes} B")
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

/// The banner for a feature-action failure (assign self, confirm
/// classification): `UnderScoped` renders the actionable treatment (a
/// "Change token…" button wired to `on_change_token`), `Plain` the same
/// banner without one. See [`FeatureActionError`].
#[component]
fn FeatureActionErrorBanner(
    error: FeatureActionError,
    on_change_token: EventHandler<()>,
) -> Element {
    match error {
        FeatureActionError::UnderScoped(message) => rsx! {
            ErrorBanner { message, on_change_token }
        },
        FeatureActionError::Plain(message) => rsx! {
            ErrorBanner { message }
        },
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
fn Board(slices: Vec<Slice>, graph_view: bool, on_assign: EventHandler<u64>) -> Element {
    let lanes = group_into_lanes(slices);
    // The board-wide "highlighted issue", shared across lanes so a dependency
    // badge can highlight its referenced card in any column. `None` when nothing
    // is hovered.
    let mut highlighted = use_signal(|| Option::<u64>::None);
    // A single, stable handler for the highlight intent. The whole card tree
    // shares this one generational box (forwarded by-value, never re-wrapped),
    // so a hover/focus event arriving during the background poll's re-render
    // never invokes a dropped box (`ValueDroppedError`). Re-creating it each
    // render — or re-wrapping it at every relay layer — would mint a fresh box
    // per render and reintroduce that race.
    let on_highlight = use_callback(move |number| highlighted.set(number));
    rsx! {
        div { class: "flex flex-col gap-6",
            for lane in lanes {
                PrdLane {
                    key: "{lane.prd.as_ref().map(|prd| prd.number).unwrap_or(0)}",
                    prd: lane.prd,
                    slices: lane.slices,
                    graph_view,
                    on_assign,
                    highlighted: highlighted(),
                    on_highlight,
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
            div { class: "collapse collapse-arrow bg-base-200 border border-base-300",
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
                                on_confirm,
                            }
                        }
                    }
                }
            }
        }
    }
}
