//! Application layer: use-cases and the port traits that infrastructure
//! implements (dependency inversion). Depends only on `domain`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use domain::{
    classify_issue, parse_blockers_from_body, parse_parent_from_body, resolve_unblocks, AppAction,
    AppError, AppResult, DependencyRef, GitHubToken, IssueClassification, Prd, PrdRef, Project,
    RawIssue, RawSlice, RepoRef, Slice,
};

/// The seam between the application and any GitHub backend (real or fake).
///
/// This is the primary test seam: use-cases run against a fake implementation
/// returning canned data, with no network access.
#[async_trait]
pub trait GitHubPort: Send + Sync {
    /// Load the Slices that make up a project's board.
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>>;

    /// Load the project's issues as raw, GitHub-shaped data, including closed
    /// ones.
    ///
    /// The application layer classifies these and builds the board from them.
    /// The adapter provides both the native-link fields (`native_parent`,
    /// `native_blockers`, `is_native_child_of_prd`) and the raw `body` for the
    /// prose-fallback parsing. Closed issues are included (`RawIssue.closed`);
    /// `classify_board` is responsible for omitting them.
    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>>;

    /// List the repositories the token can access, for the home screen. Ordering
    /// is the adapter's best effort; [`ProjectsService`] re-sorts by recency.
    async fn list_projects(&self) -> AppResult<Vec<Project>>;

    /// Assign the authenticated user to an issue (the Slice's underlying issue),
    /// so picking up a Ready Slice from the board claims it on GitHub.
    async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction;

    /// Add a label to an issue, so confirming a suggested classification tags it
    /// (`prd` or `slice`) and the next poll reclassifies it onto the board.
    async fn add_label(&self, repo: &RepoRef, issue_number: u64, label: &str) -> AppAction;
}

/// Shared ports are ports too, so the composition root can hand the same
/// `Arc<dyn GitHubPort>` to a [`BoardService`] (and clone it into contexts).
#[async_trait]
impl<P: GitHubPort + ?Sized> GitHubPort for Arc<P> {
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>> {
        (**self).load_board(repo).await
    }

    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        (**self).load_issues(repo).await
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        (**self).list_projects().await
    }

    async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction {
        (**self).assign_self(repo, issue_number).await
    }

    async fn add_label(&self, repo: &RepoRef, issue_number: u64, label: &str) -> AppAction {
        (**self).add_label(repo, issue_number, label).await
    }
}

/// An issue that is not a confirmed Slice — shown in the "other open issues"
/// section of the board.
///
/// The `classification` field drives the "looks like a PRD/Slice — confirm?"
/// badge for [`IssueClassification::SuggestedPrd`] and
/// [`IssueClassification::SuggestedSlice`] issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherIssue {
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// [`IssueClassification::SuggestedPrd`], [`IssueClassification::SuggestedSlice`],
    /// or [`IssueClassification::Unclassified`].
    pub classification: IssueClassification,
}

/// The result of classifying all open issues in a project.
///
/// - `slices` — tier-1 confirmed Slices, ready for the Kanban columns.
/// - `prds`   — tier-1 confirmed PRDs (display is deferred to a later slice).
/// - `other`  — suggested and unclassified issues for the "other open issues" bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedBoard {
    pub slices: Vec<Slice>,
    pub prds: Vec<Prd>,
    pub other: Vec<OtherIssue>,
}

/// Resolve an issue number to a [`DependencyRef`] (number + title + url) against
/// the board's identity map. Numbers absent from the board (e.g. closed or beyond
/// the fetched page) yield `None`, so they are simply not shown as a badge.
fn dependency_ref(number: u64, prd_by_number: &HashMap<u64, PrdRef>) -> Option<DependencyRef> {
    prd_by_number.get(&number).map(|prd| DependencyRef {
        number,
        title: prd.title.clone(),
        url: prd.url.clone(),
    })
}

/// The seam between the application and the OS secure store (real or fake).
///
/// `infrastructure` implements this against the operating system's credential
/// store (keyring); use-cases run against a fake in tests, so no real keyring
/// is touched.
#[async_trait]
pub trait SecureStorePort: Send + Sync {
    /// Persist the Personal Access Token, replacing any existing one.
    async fn save_token(&self, token: &GitHubToken) -> AppAction;
    /// Read the stored token, or `None` when none has been saved yet.
    async fn load_token(&self) -> AppResult<Option<GitHubToken>>;
    /// Remove the stored token, if any (signing out).
    async fn delete_token(&self) -> AppAction;
}

/// Shared stores are stores too, so the composition root can pick a store at
/// runtime (`Arc<dyn SecureStorePort>`) and hand it to an [`AuthService`].
#[async_trait]
impl<S: SecureStorePort + ?Sized> SecureStorePort for Arc<S> {
    async fn save_token(&self, token: &GitHubToken) -> AppAction {
        (**self).save_token(token).await
    }

    async fn load_token(&self) -> AppResult<Option<GitHubToken>> {
        (**self).load_token().await
    }

    async fn delete_token(&self) -> AppAction {
        (**self).delete_token().await
    }
}

/// The seam between the application and on-device persistence of which project
/// was last opened, so the app can reopen it on the next launch.
///
/// `infrastructure` implements this against a local file; use-cases run against
/// a fake in tests, so no disk is touched.
#[async_trait]
pub trait ProjectStorePort: Send + Sync {
    /// The project opened most recently, or `None` if none has been opened yet.
    async fn last_opened(&self) -> AppResult<Option<RepoRef>>;
    /// Remember `repo` as the most recently opened project.
    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction;
    /// The recent-projects list cached on the last successful fetch, or `None`
    /// when the cache is cold (nothing cached yet, or it could not be read).
    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>>;
    /// Replace the cached recent-projects list with `projects`.
    async fn cache_projects(&self, projects: &[Project]) -> AppAction;
    /// The repositories the user has summoned by name on the home screen, in
    /// newest-added-first order.
    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>>;
    /// Add a repo to the tracked list if not already present (idempotent).
    async fn track_repo(&self, repo: &RepoRef) -> AppAction;
    /// Remove a repo from the tracked list.
    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction;
}

/// Shared stores are stores too, so the composition root can hand the same
/// `Arc<dyn ProjectStorePort>` to a [`ProjectsService`].
#[async_trait]
impl<S: ProjectStorePort + ?Sized> ProjectStorePort for Arc<S> {
    async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        (**self).last_opened().await
    }

    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction {
        (**self).remember_last_opened(repo).await
    }

    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>> {
        (**self).cached_projects().await
    }

    async fn cache_projects(&self, projects: &[Project]) -> AppAction {
        (**self).cache_projects(projects).await
    }

    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>> {
        (**self).tracked_repos().await
    }

    async fn track_repo(&self, repo: &RepoRef) -> AppAction {
        (**self).track_repo(repo).await
    }

    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction {
        (**self).untrack_repo(repo).await
    }
}

/// Use-cases for the project board.
pub struct BoardService<P: GitHubPort> {
    port: P,
}

impl<P: GitHubPort> BoardService<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    /// Load the board for a project.
    pub async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>> {
        self.port
            .load_board(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))
    }

    /// Assign the authenticated user to a Ready Slice, claiming it on GitHub.
    ///
    /// On success the caller re-polls the board: the now-assigned Slice derives
    /// `Wip` and leaves the Ready column. On failure the error carries the repo
    /// and issue for context and the board is left unchanged.
    pub async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction {
        self.port
            .assign_self(repo, issue_number)
            .await
            .map_err(|err| {
                err.with_context("repo", repo)
                    .with_context("issue", issue_number)
            })
    }

    /// Confirm a *suggested* classification by adding its tier-1 label (`prd` or
    /// `slice`) to the issue on GitHub.
    ///
    /// The label is derived from the suggestion via
    /// [`IssueClassification::suggested_label`]; a classification with nothing to
    /// confirm (already confident, or unclassified) is rejected as invalid input
    /// before any GitHub call. On success the caller re-polls the board: the
    /// now-labelled issue classifies tier-1 and leaves the "other open issues"
    /// bucket. On failure the error carries the repo and issue for context and
    /// the issue is left unchanged.
    pub async fn confirm_classification(
        &self,
        repo: &RepoRef,
        issue_number: u64,
        classification: &IssueClassification,
    ) -> AppAction {
        let label = classification.suggested_label().ok_or_else(|| {
            AppError::invalid_input("This issue has no suggested classification to confirm.")
                .with_context("repo", repo)
                .with_context("issue", issue_number)
        })?;

        self.port
            .add_label(repo, issue_number, label)
            .await
            .map_err(|err| {
                err.with_context("repo", repo)
                    .with_context("issue", issue_number)
                    .with_context("label", label)
            })
    }

    /// Load and classify all open issues for a project, returning the board
    /// split into confirmed Slices and an "other open issues" bucket.
    ///
    /// Closed issues are omitted. Confident-tier-1 Slices appear in
    /// [`ClassifiedBoard::slices`]; confident PRDs appear in
    /// [`ClassifiedBoard::prds`]; suggested and unclassified issues appear in
    /// [`ClassifiedBoard::other`].
    pub async fn classify_board(&self, repo: &RepoRef) -> AppResult<ClassifiedBoard> {
        let raw_issues = self
            .port
            .load_issues(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))?;

        // The numbers of issues that are still open in this board, so prose
        // blockers (which carry no open/closed state of their own) can be
        // filtered to open ones only — mirroring the still-open guarantee that
        // native blockers already have, and avoiding a false Blocked state.
        let open_numbers: HashSet<u64> = raw_issues
            .iter()
            .filter(|raw| !raw.closed)
            .map(|raw| raw.number)
            .collect();

        // Identity of every issue in the board, so a Slice's parent reference
        // (native or prose) resolves to its PRD ref (number + title + url) for
        // the swimlane header and link.
        let prd_by_number: HashMap<u64, PrdRef> = raw_issues
            .iter()
            .map(|raw| {
                (
                    raw.number,
                    PrdRef {
                        number: raw.number,
                        title: raw.title.clone(),
                        url: raw.url.clone(),
                    },
                )
            })
            .collect();

        let mut slices = Vec::new();
        let mut prds = Vec::new();
        let mut other = Vec::new();

        for raw in raw_issues {
            if raw.closed {
                continue;
            }

            match classify_issue(&raw) {
                IssueClassification::Slice => {
                    let body_str = raw.body.as_deref().unwrap_or("");
                    // Use native blockers (already still-open) when present;
                    // otherwise fall back to prose, keeping only blockers that
                    // are still open in this board. Each is resolved to its ref
                    // (number + url) against the board for the blocker badges.
                    let blockers: Vec<DependencyRef> = if !raw.native_blockers.is_empty() {
                        raw.native_blockers
                            .iter()
                            .filter_map(|number| dependency_ref(*number, &prd_by_number))
                            .collect()
                    } else {
                        parse_blockers_from_body(body_str)
                            .into_iter()
                            .filter(|number| open_numbers.contains(number))
                            .filter_map(|number| dependency_ref(number, &prd_by_number))
                            .collect()
                    };
                    // Resolve the parent PRD: prefer the native parent link,
                    // fall back to the prose `## Parent` reference, then look the
                    // number up among the issues in this board.
                    let prd = raw
                        .native_parent
                        .or_else(|| parse_parent_from_body(body_str))
                        .and_then(|number| prd_by_number.get(&number).cloned());
                    slices.push(RawSlice {
                        number: raw.number,
                        title: raw.title,
                        url: raw.url,
                        closed: false,
                        prd,
                        assignee: raw.assignee,
                        has_open_linked_pr: raw.has_open_linked_pr,
                        blockers,
                        // Filled by `resolve_unblocks` once the board is mapped.
                        unblocks: Vec::new(),
                    });
                }
                IssueClassification::Prd => {
                    prds.push(Prd {
                        number: raw.number,
                        title: raw.title,
                    });
                }
                classification => {
                    other.push(OtherIssue {
                        number: raw.number,
                        title: raw.title,
                        url: raw.url,
                        classification,
                    });
                }
            }
        }

        // Derive the reverse "unblocks" edge across the whole board, then project
        // each raw Slice into its read model with the derived state.
        resolve_unblocks(&mut slices);
        let slices = slices.into_iter().map(RawSlice::into_slice).collect();

        Ok(ClassifiedBoard {
            slices,
            prds,
            other,
        })
    }
}

/// Use-cases for Personal Access Token authentication, backed by a
/// [`SecureStorePort`].
pub struct AuthService<S: SecureStorePort> {
    store: S,
}

impl<S: SecureStorePort> AuthService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Validate a pasted PAT and persist it to the OS secure store, so it is
    /// reused across launches. Rejects anything that is not a fine-grained PAT
    /// before it ever reaches the store.
    pub async fn save_token(&self, raw: &str) -> AppAction {
        let token = GitHubToken::parse(raw)?;
        self.store
            .save_token(&token)
            .await
            .map_err(|err| err.with_operation("AuthService::save_token"))
    }

    /// The stored token, or an `Unauthorized` error when none is saved yet.
    ///
    /// Callers use the `Unauthorized` case to route the user to the paste-token
    /// screen.
    pub async fn require_token(&self) -> AppResult<GitHubToken> {
        self.store
            .load_token()
            .await
            .map_err(|err| err.with_operation("AuthService::require_token"))?
            .ok_or_else(|| {
                AppError::unauthorized("Add a Personal Access Token to load your board.")
                    .with_operation("AuthService::require_token")
            })
    }

    /// Whether a token is already stored, to decide the launch screen.
    pub async fn has_token(&self) -> AppResult<bool> {
        Ok(self
            .store
            .load_token()
            .await
            .map_err(|err| err.with_operation("AuthService::has_token"))?
            .is_some())
    }

    /// Remove the stored token (sign out).
    pub async fn clear_token(&self) -> AppAction {
        self.store
            .delete_token()
            .await
            .map_err(|err| err.with_operation("AuthService::clear_token"))
    }
}

/// Use-case for the home screen: listing the accessible projects,
/// most-recently-pushed first, backed by a [`GitHubPort`].
pub struct ProjectsService<G: GitHubPort> {
    github: G,
}

impl<G: GitHubPort> ProjectsService<G> {
    pub fn new(github: G) -> Self {
        Self { github }
    }

    /// The accessible projects, most-recently-pushed first.
    ///
    /// Ordering is owned here rather than trusted from the adapter: `pushed_at`
    /// is an RFC-3339 UTC timestamp, so a descending lexical sort puts the most
    /// recently active project first.
    pub async fn recent_projects(&self) -> AppResult<Vec<Project>> {
        let mut projects = self
            .github
            .list_projects()
            .await
            .map_err(|err| err.with_operation("ProjectsService::recent_projects"))?;
        projects.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
        Ok(projects)
    }
}

/// The outcome of a stale-while-revalidate refresh of the recent-projects list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectsRefresh {
    /// The live list differs from the cache (including a cold cache): the cache
    /// has been rewritten and the UI should swap to this list.
    Changed(Vec<Project>),
    /// The live list matches the cache: nothing was written and the UI should
    /// stay put (no flicker).
    Unchanged,
}

/// Use-case for the home screen's recent-projects list with
/// **stale-while-revalidate** caching: the cached list renders instantly while a
/// live fetch refreshes it — and the local cache — only when it actually
/// changed.
///
/// It composes the recency-sorted live fetch ([`ProjectsService`]) with the
/// on-device cache ([`ProjectStorePort`]), so the recency ordering stays owned in
/// one place and the change decision is testable against fakes (no disk, no live
/// GitHub).
pub struct RecentProjectsService<G: GitHubPort, S: ProjectStorePort> {
    projects: ProjectsService<G>,
    cache: S,
}

impl<G: GitHubPort, S: ProjectStorePort> RecentProjectsService<G, S> {
    pub fn new(github: G, cache: S) -> Self {
        Self {
            projects: ProjectsService::new(github),
            cache,
        }
    }

    /// The cached list for an instant first paint, or `None` on a cold cache.
    /// A local read only: no token or network involved.
    pub async fn cached(&self) -> AppResult<Option<Vec<Project>>> {
        self.cache
            .cached_projects()
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::cached"))
    }

    /// Fetch the live list (recency-sorted), compare it with the cache, and
    /// persist + report it only when it changed; an unchanged list leaves the
    /// cache untouched and reports [`ProjectsRefresh::Unchanged`].
    pub async fn refresh(&self) -> AppResult<ProjectsRefresh> {
        let live = self.projects.recent_projects().await?;
        let cached = self
            .cache
            .cached_projects()
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::refresh"))?;
        if cached.as_deref() == Some(live.as_slice()) {
            return Ok(ProjectsRefresh::Unchanged);
        }
        self.cache
            .cache_projects(&live)
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::refresh"))?;
        Ok(ProjectsRefresh::Changed(live))
    }
}

/// Use-cases for remembering and reopening the last-opened project, backed by a
/// [`ProjectStorePort`]. A purely local concern: unlike [`ProjectsService`] it
/// needs neither a token nor the network, so it never fails for auth reasons.
pub struct LastOpenedService<S: ProjectStorePort> {
    store: S,
}

impl<S: ProjectStorePort> LastOpenedService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Remember `repo` as the most recently opened project, so the next launch
    /// reopens it.
    pub async fn open_project(&self, repo: &RepoRef) -> AppAction {
        self.store
            .remember_last_opened(repo)
            .await
            .map_err(|err| err.with_operation("LastOpenedService::open_project"))
    }

    /// The project to reopen on launch, or `None` to show the home screen.
    pub async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        self.store
            .last_opened()
            .await
            .map_err(|err| err.with_operation("LastOpenedService::last_opened"))
    }
}

/// Use-case for opening a project summoned by name on the home screen (the
/// go-to action), composing the live board fetch ([`BoardService`]) with the
/// on-device tracked/last-opened store ([`ProjectStorePort`]).
///
/// Keeping this orchestration here — rather than in the presentation layer —
/// makes the "track only on a successful open" behaviour testable at the
/// use-case seam against fakes (no GitHub, no disk).
pub struct TrackedProjectsService<G: GitHubPort, S: ProjectStorePort> {
    board: BoardService<G>,
    store: S,
}

impl<G: GitHubPort, S: ProjectStorePort> TrackedProjectsService<G, S> {
    pub fn new(github: G, store: S) -> Self {
        Self {
            board: BoardService::new(github),
            store,
        }
    }

    /// Open a repo summoned by name: load and classify its board to verify
    /// access, then — only on success — track the repo (idempotent) and
    /// remember it as last-opened before returning the board. A failed load
    /// (e.g. a 404 for a repo that does not exist or is not accessible)
    /// propagates and tracks nothing.
    ///
    /// The track and last-opened writes are deliberately **best-effort**: the
    /// board has already loaded, so a local-store write failure must not fail an
    /// otherwise-successful open. Such a failure simply leaves the repo
    /// untracked for now — the next successful open re-attempts it — rather than
    /// surfacing an error to the user.
    pub async fn open_and_track(&self, repo: &RepoRef) -> AppResult<ClassifiedBoard> {
        let board = self.board.classify_board(repo).await?;
        let _ = self.store.track_repo(repo).await;
        let _ = self.store.remember_last_opened(repo).await;
        Ok(board)
    }
}
