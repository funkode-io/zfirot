//! The real [`GitHubPort`] adapter: one GraphQL query per board load.
//!
//! Relationships are read from GitHub's **native** links first — the sub-issue
//! `parent` for the PRD, and `blockedBy` dependencies for the Blocked state —
//! and fall back to parsing the issue body's `## Parent` / `## Blocked by` prose
//! when those native links are absent. Prose references are resolved against the
//! issues fetched in the same load, so a prose parent yields the real PRD title
//! and only prose blockers that are still open count toward Blocked. The HTTP
//! boundary is kept thin; the payload projection lives in the pure
//! [`parse_response`] / [`resolve_board`] functions so it is testable offline.

use application::GitHubPort;
use async_trait::async_trait;
use domain::{parse_prose, AppError, AppResult, Project, ProseLinks, RawSlice, RepoRef, Slice};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// One page of issues, with the native relationships the board derives state
/// from. `states: OPEN` because the board never shows Done (closed) Slices.
const BOARD_QUERY: &str = r#"
query Board($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, after: $cursor, states: OPEN, orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        title
        url
        body
        assignees(first: 1) { nodes { login } }
        parent { title }
        blockedBy(first: 50) { nodes { state } }
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { state } }
      }
    }
  }
}
"#;

/// The viewer's accessible repositories, most-recently-pushed first, for the
/// home screen. One page of up to 50 is plenty for a recent-projects list;
/// `ProjectsService` re-sorts by `pushedAt` regardless of the returned order.
const PROJECTS_QUERY: &str = r#"
query Projects($cursor: String) {
  viewer {
    repositories(
      first: 50,
      after: $cursor,
      orderBy: {field: PUSHED_AT, direction: DESC},
      affiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER]
    ) {
      pageInfo { hasNextPage endCursor }
      nodes {
        name
        pushedAt
        isFork
        owner { login }
        parent {
          name
          pushedAt
          owner { login }
        }
      }
    }
  }
}
"#;

/// A GitHub GraphQL adapter. The token is injected by the composition root (the
/// adapter never reads the environment itself) and held only inside the HTTP
/// client's default `Authorization` header, marked sensitive so it is not logged.
pub struct GitHubClient {
    http: reqwest::Client,
    endpoint: String,
}

impl GitHubClient {
    /// Build a client from an already-resolved token.
    ///
    /// The token and user-agent are baked into the client's default headers, so
    /// every request is authenticated without re-supplying them (and the token
    /// lives only in the sensitive header, not in a plain field).
    pub fn new(token: impl AsRef<str>) -> AppResult<Self> {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", token.as_ref()))
            .map_err(|err| {
                AppError::invalid_input("The GitHub token contains invalid characters.")
                    .with_operation("GitHubClient::new")
                    .with_source(err)
            })?;
        authorization.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(USER_AGENT, HeaderValue::from_static("zfirot"));

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|err| {
                AppError::internal("Could not build the GitHub HTTP client")
                    .with_operation("GitHubClient::new")
                    .with_source(err)
            })?;

        Ok(Self {
            http,
            endpoint: GITHUB_GRAPHQL_URL.to_string(),
        })
    }

    /// Fetch a single page of issues for `repo`, starting after `cursor`.
    async fn fetch_page(&self, repo: &RepoRef, cursor: Option<&str>) -> AppResult<String> {
        let body = serde_json::json!({
            "query": BOARD_QUERY,
            "variables": { "owner": repo.owner, "name": repo.name, "cursor": cursor },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_page")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status, &response, "GitHubClient::fetch_page"));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_page")
                .with_source(err)
        })
    }

    /// Fetch a single page of the viewer's repositories, starting after `cursor`.
    async fn fetch_projects_page(&self, cursor: Option<&str>) -> AppResult<String> {
        let body = serde_json::json!({
            "query": PROJECTS_QUERY,
            "variables": { "cursor": cursor },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_projects_page")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_projects_page",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_projects_page")
                .with_source(err)
        })
    }
}

#[async_trait]
impl GitHubPort for GitHubClient {
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>> {
        let mut issues = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self.fetch_page(repo, cursor.as_deref()).await?;
            let (page, next) = parse_response(&body)?;
            issues.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(resolve_board(issues)
            .into_iter()
            .map(RawSlice::into_slice)
            .collect())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        let mut projects = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self.fetch_projects_page(cursor.as_deref()).await?;
            let (page, next) = parse_projects_response(&body)?;
            projects.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(projects)
    }
}

/// Map a GitHub `4xx/5xx` response to an [`AppError`] the caller can act on.
/// `operation` names the calling fetch so diagnostics point at the right one
/// (board vs. project listing).
fn status_error(
    status: reqwest::StatusCode,
    response: &reqwest::Response,
    operation: &'static str,
) -> AppError {
    let rate_limited = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim() == "0")
        .unwrap_or(false);

    match status.as_u16() {
        401 => AppError::unauthorized("GitHub rejected the token").with_operation(operation),
        403 if rate_limited => {
            AppError::rate_limited("GitHub rate limit exceeded").with_operation(operation)
        }
        403 => AppError::forbidden("The token lacks access to this repository")
            .with_operation(operation),
        // GitHub-side failures the caller can only retry later.
        500..=599 => AppError::unavailable("GitHub is temporarily unavailable")
            .with_operation(operation)
            .with_context("status", status),
        // Any other status means our request was wrong: a bug, not a transient.
        _ => AppError::internal("GitHub returned an unexpected status")
            .with_operation(operation)
            .with_context("status", status),
    }
}

/// Parse a GraphQL response body into a page of [`RawIssue`]s and the cursor of
/// the next page (if any). Pure and offline: the primary test seam. Prose
/// references are resolved later by [`resolve_board`], once every page is in.
pub fn parse_response(body: &str) -> AppResult<(Vec<RawIssue>, Option<String>)> {
    let response: GraphQlResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_response")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        let not_found = errors.iter().any(|error| {
            error.error_type.as_deref() == Some("NOT_FOUND")
                || error
                    .message
                    .to_lowercase()
                    .contains("could not resolve to a repository")
        });
        let message = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        let lowered = message.to_lowercase();
        // A repository GitHub reports via the `errors` array (e.g. private or
        // renamed) is still a NotFound, not an Internal failure.
        let error = if not_found {
            AppError::not_found("Repository not found or not visible to the token")
        } else if lowered.contains("rate limit") {
            AppError::rate_limited("GitHub rate limit exceeded")
        } else {
            AppError::internal("GitHub reported a query error")
        };
        return Err(error
            .with_operation("parse_response")
            .with_context("errors", message));
    }

    let repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| {
            AppError::not_found("Repository not found or not visible to the token")
                .with_operation("parse_response")
        })?;

    let issues = repository.issues;
    let raw = issues.nodes.into_iter().map(map_issue).collect();
    let next = if issues.page_info.has_next_page {
        issues.page_info.end_cursor
    } else {
        None
    };

    Ok((raw, next))
}

/// Parse a GraphQL projects response into a page of [`Project`]s and the cursor
/// of the next page (if any). Pure and offline, mirroring [`parse_response`].
pub fn parse_projects_response(body: &str) -> AppResult<(Vec<Project>, Option<String>)> {
    let response: ProjectsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_projects_response")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        let message = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        let error = if message.to_lowercase().contains("rate limit") {
            AppError::rate_limited("GitHub rate limit exceeded")
        } else {
            AppError::internal("GitHub reported a query error")
        };
        return Err(error
            .with_operation("parse_projects_response")
            .with_context("errors", message));
    }

    let repositories = response
        .data
        .map(|data| data.viewer.repositories)
        .ok_or_else(|| {
            AppError::internal("GitHub returned no viewer data")
                .with_operation("parse_projects_response")
        })?;

    // Resolve each node to the project the app actually tracks: a fork stands in
    // for its upstream parent (issues live upstream, not on the fork), so we map
    // forks to their parent's identity and recency. Mapping can collapse two
    // nodes onto the same upstream (e.g. an org repo plus a personal fork of it),
    // so we de-duplicate by repository, keeping the most recent push.
    let mut by_repo: Vec<Project> = Vec::with_capacity(repositories.nodes.len());
    for node in repositories.nodes {
        let project = node_into_project(node);
        match by_repo.iter_mut().find(|seen| seen.repo == project.repo) {
            Some(seen) if project.pushed_at > seen.pushed_at => seen.pushed_at = project.pushed_at,
            Some(_) => {}
            None => by_repo.push(project),
        }
    }
    let projects = by_repo;

    let next = if repositories.page_info.has_next_page {
        repositories.page_info.end_cursor
    } else {
        None
    };

    Ok((projects, next))
}

/// Map a repository node to the project the app tracks. A fork stands in for its
/// upstream parent: the board reads issues from upstream, so we adopt the
/// parent's owner/name and its push time (the project's real activity). A
/// non-fork (or a fork whose parent the token cannot see) keeps its own
/// identity. A null `pushedAt` becomes an empty string, which sorts last.
fn node_into_project(node: RepositoryNode) -> Project {
    match node.parent {
        Some(parent) if node.is_fork => Project::new(
            RepoRef::new(parent.owner.login, parent.name),
            parent.pushed_at.unwrap_or_default(),
        ),
        _ => Project::new(
            RepoRef::new(node.owner.login, node.name),
            node.pushed_at.unwrap_or_default(),
        ),
    }
}

/// Resolve native-or-prose relationships across a whole board into [`RawSlice`]s.
///
/// Native links win; when an issue has none, its parsed prose references are
/// resolved against the issues fetched in the same load: a prose parent yields
/// that issue's title for the PRD tag, and a prose blocker counts toward Blocked
/// only if it is still open (open issues are the only ones in the fetched set).
pub fn resolve_board(issues: Vec<RawIssue>) -> Vec<RawSlice> {
    let open_numbers: HashSet<u64> = issues.iter().map(|issue| issue.number).collect();
    let title_by_number: HashMap<u64, String> = issues
        .iter()
        .map(|issue| (issue.number, issue.title.clone()))
        .collect();

    issues
        .into_iter()
        .map(|issue| resolve_issue(issue, &open_numbers, &title_by_number))
        .collect()
}

/// Project one [`RawIssue`] into a [`RawSlice`], preferring native links and
/// falling back to its prose references resolved against the fetched board.
fn resolve_issue(
    issue: RawIssue,
    open_numbers: &HashSet<u64>,
    title_by_number: &HashMap<u64, String>,
) -> RawSlice {
    let prd_title = match issue.native_parent {
        Some(parent) => Some(parent.title),
        None => issue
            .prose
            .parent
            .and_then(|number| title_by_number.get(&number).cloned()),
    };

    let open_blocker_count = if issue.native_blocker_states.is_empty() {
        issue
            .prose
            .blocked_by
            .iter()
            .filter(|number| open_numbers.contains(number))
            .count() as u32
    } else {
        issue
            .native_blocker_states
            .iter()
            .filter(|state| state.as_str() == "OPEN")
            .count() as u32
    };

    RawSlice {
        number: issue.number,
        title: issue.title,
        url: issue.url,
        closed: false,
        prd_title,
        assignee: issue.assignee,
        has_open_linked_pr: issue.has_open_linked_pr,
        open_blocker_count,
    }
}

/// Project one GraphQL issue node into a [`RawIssue`]: its native facts plus the
/// prose relationships parsed from its body, to be resolved by [`resolve_board`].
/// Only open issues are queried, so a closed issue never reaches this mapping.
fn map_issue(node: IssueNode) -> RawIssue {
    let has_open_linked_pr = node
        .closed_by_pull_requests_references
        .nodes
        .iter()
        .any(|pr| pr.state == "OPEN");

    RawIssue {
        number: node.number,
        title: node.title,
        url: node.url,
        assignee: node
            .assignees
            .nodes
            .into_iter()
            .next()
            .map(|user| user.login),
        has_open_linked_pr,
        native_parent: node.parent.map(|parent| NativeParent {
            title: parent.title,
        }),
        native_blocker_states: node
            .blocked_by
            .nodes
            .into_iter()
            .map(|blocker| blocker.state)
            .collect(),
        prose: parse_prose(&node.body),
    }
}

/// A single issue's native facts plus the prose relationships parsed from its
/// body, before references are resolved against the rest of the board.
#[derive(Debug)]
pub struct RawIssue {
    number: u64,
    title: String,
    url: String,
    assignee: Option<String>,
    has_open_linked_pr: bool,
    native_parent: Option<NativeParent>,
    native_blocker_states: Vec<String>,
    prose: ProseLinks,
}

/// The native sub-issue parent of an issue, carrying the PRD title to tag with.
#[derive(Debug)]
struct NativeParent {
    title: String,
}

#[derive(Deserialize)]
struct GraphQlResponse {
    data: Option<ResponseData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

#[derive(Deserialize)]
struct ResponseData {
    repository: Option<RepositoryData>,
}

#[derive(Deserialize)]
struct RepositoryData {
    issues: IssueConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueConnection {
    page_info: PageInfo,
    nodes: Vec<IssueNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueNode {
    number: u64,
    title: String,
    url: String,
    body: String,
    assignees: LoginConnection,
    parent: Option<ParentIssue>,
    blocked_by: StateConnection,
    closed_by_pull_requests_references: StateConnection,
}

#[derive(Deserialize)]
struct LoginConnection {
    nodes: Vec<Login>,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct ParentIssue {
    title: String,
}

#[derive(Deserialize)]
struct StateConnection {
    nodes: Vec<StateNode>,
}

#[derive(Deserialize)]
struct StateNode {
    state: String,
}

#[derive(Deserialize)]
struct ProjectsResponse {
    data: Option<ProjectsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct ProjectsData {
    viewer: Viewer,
}

#[derive(Deserialize)]
struct Viewer {
    repositories: RepositoryConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryConnection {
    page_info: PageInfo,
    nodes: Vec<RepositoryNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryNode {
    name: String,
    pushed_at: Option<String>,
    #[serde(default)]
    is_fork: bool,
    owner: RepositoryOwner,
    parent: Option<ParentRepositoryNode>,
}

/// A fork's upstream repository, the project the app actually tracks.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentRepositoryNode {
    name: String,
    pushed_at: Option<String>,
    owner: RepositoryOwner,
}

#[derive(Deserialize)]
struct RepositoryOwner {
    login: String,
}
