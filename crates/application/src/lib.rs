//! Application layer: use-cases and the port traits that infrastructure
//! implements (dependency inversion). Depends only on `domain`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use domain::{
    classify_issue, parse_blockers_from_body, parse_parent_from_body, AppAction, AppError,
    AppResult, GitHubToken, IssueClassification, Prd, Project, RawIssue, RawSlice, RepoRef, Slice,
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

        // Title of every issue in the board, so a Slice's parent reference
        // (native or prose) resolves to its PRD title for the card tag.
        let title_by_number: HashMap<u64, String> = raw_issues
            .iter()
            .map(|raw| (raw.number, raw.title.clone()))
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
                    // are still open in this board.
                    let open_blocker_count = if !raw.native_blockers.is_empty() {
                        raw.native_blockers.len() as u32
                    } else {
                        parse_blockers_from_body(body_str)
                            .into_iter()
                            .filter(|number| open_numbers.contains(number))
                            .count() as u32
                    };
                    // Resolve the parent PRD title: prefer the native parent
                    // link, fall back to the prose `## Parent` reference, then
                    // look the number up among the issues in this board.
                    let prd_title = raw
                        .native_parent
                        .or_else(|| parse_parent_from_body(body_str))
                        .and_then(|number| title_by_number.get(&number).cloned());
                    let raw_slice = RawSlice {
                        number: raw.number,
                        title: raw.title,
                        url: raw.url,
                        closed: false,
                        prd_title,
                        assignee: raw.assignee,
                        has_open_linked_pr: raw.has_open_linked_pr,
                        open_blocker_count,
                    };
                    slices.push(raw_slice.into_slice());
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
