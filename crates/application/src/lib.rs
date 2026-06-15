//! Application layer: use-cases and the port traits that infrastructure
//! implements (dependency inversion). Depends only on `domain`.

use async_trait::async_trait;
use domain::{
    classify_issue, parse_blockers_from_body, parse_parent_from_body, AppResult,
    IssueClassification, Prd, RawIssue, RawSlice, RepoRef, Slice,
};

/// The seam between the application and any GitHub backend (real or fake).
///
/// This is the primary test seam: use-cases run against a fake implementation
/// returning canned data, with no network access.
#[async_trait]
pub trait GitHubPort: Send + Sync {
    /// Load the Slices that make up a project's board.
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>>;

    /// Load all open issues for a project as raw, unclassified GitHub data.
    ///
    /// The application layer classifies the issues and builds the board from
    /// this data. The adapter is responsible for providing both native-link
    /// fields (`native_parent`, `native_blockers`, `is_native_child_of_prd`) and
    /// the raw `body` for prose-fallback parsing.
    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>>;
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
                    // Use native blockers when present; fall back to prose parsing.
                    let open_blocker_count = if !raw.native_blockers.is_empty() {
                        raw.native_blockers.len() as u32
                    } else {
                        parse_blockers_from_body(body_str).len() as u32
                    };
                    // Resolve parent number from native link or prose fallback.
                    // Title resolution against the PRD list is deferred; `prd_title`
                    // is left `None` here and will be filled in a later slice.
                    let _effective_parent = raw
                        .native_parent
                        .or_else(|| parse_parent_from_body(body_str));
                    let raw_slice = RawSlice {
                        number: raw.number,
                        title: raw.title,
                        closed: false,
                        prd_title: None,
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
