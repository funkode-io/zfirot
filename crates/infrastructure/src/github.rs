//! The real [`GitHubPort`] adapter: one GraphQL query per board load.
//!
//! Relationships are read from GitHub's **native** links only — the sub-issue
//! `parent` for the PRD, and `blockedBy` dependencies for the Blocked state. A
//! prose fallback (parsing `## Parent` / `## Blocked by` issue bodies) is a
//! separate slice. The HTTP boundary is kept thin; the payload-to-[`RawSlice`]
//! projection lives in the pure [`parse_response`] function so it is testable
//! offline against a recorded fixture.

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppError, AppResult, RawIssue, RawSlice, RepoRef, Slice};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

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
        assignees(first: 1) { nodes { login } }
        parent { title }
        blockedBy(first: 50) { nodes { state } }
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { state } }
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
            return Err(status_error(status, &response));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_page")
                .with_source(err)
        })
    }
}

#[async_trait]
impl GitHubPort for GitHubClient {
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>> {
        let mut raw = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self.fetch_page(repo, cursor.as_deref()).await?;
            let (page, next) = parse_response(&body)?;
            raw.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(raw.into_iter().map(RawSlice::into_slice).collect())
    }

    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        // TODO: implement a dedicated GraphQL query for raw issue classification.
        // Returns an empty list until a proper `load_issues` query is added.
        Ok(vec![])
    }
}

/// Map a GitHub `4xx/5xx` response to an [`AppError`] the caller can act on.
fn status_error(status: reqwest::StatusCode, response: &reqwest::Response) -> AppError {
    let rate_limited = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim() == "0")
        .unwrap_or(false);

    match status.as_u16() {
        401 => AppError::unauthorized("GitHub rejected the token")
            .with_operation("GitHubClient::fetch_page"),
        403 if rate_limited => AppError::rate_limited("GitHub rate limit exceeded")
            .with_operation("GitHubClient::fetch_page"),
        403 => AppError::forbidden("The token lacks access to this repository")
            .with_operation("GitHubClient::fetch_page"),
        // GitHub-side failures the caller can only retry later.
        500..=599 => AppError::unavailable("GitHub is temporarily unavailable")
            .with_operation("GitHubClient::fetch_page")
            .with_context("status", status),
        // Any other status means our request was wrong: a bug, not a transient.
        _ => AppError::internal("GitHub returned an unexpected status")
            .with_operation("GitHubClient::fetch_page")
            .with_context("status", status),
    }
}

/// Parse a GraphQL response body into a page of [`RawSlice`]s and the cursor of
/// the next page (if any). Pure and offline: the primary test seam.
pub fn parse_response(body: &str) -> AppResult<(Vec<RawSlice>, Option<String>)> {
    let response: GraphQlResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_response")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        let message = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        let lowered = message.to_lowercase();
        let error = if lowered.contains("rate limit") {
            AppError::rate_limited("GitHub rate limit exceeded").with_context("errors", message)
        } else {
            AppError::internal("GitHub reported a query error").with_context("errors", message)
        };
        return Err(error.with_operation("parse_response"));
    }

    let repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| {
            AppError::not_found("Repository not found or not visible to the token")
                .with_operation("parse_response")
        })?;

    let issues = repository.issues;
    let slices = issues.nodes.into_iter().map(map_issue).collect();
    let next = if issues.page_info.has_next_page {
        issues.page_info.end_cursor
    } else {
        None
    };

    Ok((slices, next))
}

/// Project one GraphQL issue node into a [`RawSlice`]. Only open issues are
/// queried, so `closed` is always `false` here.
fn map_issue(node: IssueNode) -> RawSlice {
    let open_blocker_count = node
        .blocked_by
        .nodes
        .iter()
        .filter(|issue| issue.state == "OPEN")
        .count() as u32;

    let has_open_linked_pr = node
        .closed_by_pull_requests_references
        .nodes
        .iter()
        .any(|pr| pr.state == "OPEN");

    RawSlice {
        number: node.number,
        title: node.title,
        closed: false,
        prd_title: node.parent.map(|parent| parent.title),
        assignee: node
            .assignees
            .nodes
            .into_iter()
            .next()
            .map(|user| user.login),
        has_open_linked_pr,
        open_blocker_count,
    }
}

#[derive(Deserialize)]
struct GraphQlResponse {
    data: Option<ResponseData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
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
