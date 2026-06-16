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
use domain::{
    parse_prose, AppAction, AppError, AppResult, DependencyRef, PrdRef, Project, ProseLinks,
    RawIssue, RawSlice, RepoRef, Slice,
};
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
        parent { number title url }
        blockedBy(first: 50) { nodes { number title url state } }
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { state } }
      }
    }
  }
}
"#;

/// One page of issues for classification: **open and closed**, with the labels,
/// native relationships, and linked-PR state the two-tier classifier needs.
/// `classify_board` filters closed issues out, but they are still fetched so a
/// Slice's native parent or blocker that happens to be closed is visible to the
/// mapping (and a closed blocker is correctly dropped, not silently missing).
const ISSUES_QUERY: &str = r#"
query Issues($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, after: $cursor, states: [OPEN, CLOSED], orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        title
        url
        body
        state
        labels(first: 20) { nodes { name } }
        assignees(first: 1) { nodes { login } }
        parent { number labels(first: 20) { nodes { name } } }
        blockedBy(first: 50) { nodes { number state } }
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

/// Resolve the node IDs the assign-self mutation needs: the authenticated
/// user (`viewer`) and the target issue (the assignable). Both are looked up in
/// one round trip before the mutation runs.
const ASSIGN_IDS_QUERY: &str = r#"
query AssignIds($owner: String!, $name: String!, $number: Int!) {
  viewer { id }
  repository(owner: $owner, name: $name) {
    issue(number: $number) { id }
  }
}
"#;

/// Assign the authenticated user to an issue, claiming a Ready Slice. The board
/// re-polls after this succeeds, so the now-assigned Slice derives `Wip`.
const ASSIGN_MUTATION: &str = r#"
mutation Assign($assignableId: ID!, $assigneeId: ID!) {
  addAssigneesToAssignable(input: {assignableId: $assignableId, assigneeIds: [$assigneeId]}) {
    clientMutationId
  }
}
"#;

/// Resolve the node IDs the add-label mutation needs: the target issue (the
/// labelable) and the repository label to add it. Both are looked up in one
/// round trip before the mutation runs. A label the repository does not define
/// comes back `null`, which the parser maps to a clear NotFound.
const LABEL_IDS_QUERY: &str = r#"
query LabelIds($owner: String!, $name: String!, $number: Int!, $label: String!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) { id }
    label(name: $label) { id }
  }
}
"#;

/// Add a classifying label to an issue, confirming a suggested classification.
/// The board re-polls after this succeeds, so the now-labelled issue is
/// reclassified tier-1 (`prd` or `slice`) and leaves "other open issues".
const ADD_LABEL_MUTATION: &str = r#"
mutation AddLabel($labelableId: ID!, $labelId: ID!) {
  addLabelsToLabelable(input: {labelableId: $labelableId, labelIds: [$labelId]}) {
    clientMutationId
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

    /// Fetch a single page of issues for classification (open and closed) for
    /// `repo`, starting after `cursor`.
    async fn fetch_issues_page(&self, repo: &RepoRef, cursor: Option<&str>) -> AppResult<String> {
        let body = serde_json::json!({
            "query": ISSUES_QUERY,
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
                    .with_operation("GitHubClient::fetch_issues_page")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_issues_page",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_issues_page")
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

    /// Resolve the viewer and issue node IDs the assign mutation needs.
    async fn fetch_assign_ids(&self, repo: &RepoRef, issue_number: u64) -> AppResult<String> {
        let body = serde_json::json!({
            "query": ASSIGN_IDS_QUERY,
            "variables": { "owner": repo.owner, "name": repo.name, "number": issue_number },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_assign_ids")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_assign_ids",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_assign_ids")
                .with_source(err)
        })
    }

    /// Run the `addAssigneesToAssignable` mutation for the resolved node IDs.
    async fn run_assign_mutation(
        &self,
        assignable_id: &str,
        assignee_id: &str,
    ) -> AppResult<String> {
        let body = serde_json::json!({
            "query": ASSIGN_MUTATION,
            "variables": { "assignableId": assignable_id, "assigneeId": assignee_id },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::run_assign_mutation")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::run_assign_mutation",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::run_assign_mutation")
                .with_source(err)
        })
    }

    /// Resolve the issue and label node IDs the add-label mutation needs.
    async fn fetch_label_ids(
        &self,
        repo: &RepoRef,
        issue_number: u64,
        label: &str,
    ) -> AppResult<String> {
        let body = serde_json::json!({
            "query": LABEL_IDS_QUERY,
            "variables": {
                "owner": repo.owner, "name": repo.name, "number": issue_number, "label": label,
            },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_label_ids")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_label_ids",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_label_ids")
                .with_source(err)
        })
    }

    /// Run the `addLabelsToLabelable` mutation for the resolved node IDs.
    async fn run_add_label_mutation(
        &self,
        labelable_id: &str,
        label_id: &str,
    ) -> AppResult<String> {
        let body = serde_json::json!({
            "query": ADD_LABEL_MUTATION,
            "variables": { "labelableId": labelable_id, "labelId": label_id },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::run_add_label_mutation")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::run_add_label_mutation",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::run_add_label_mutation")
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

    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        let mut issues = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self.fetch_issues_page(repo, cursor.as_deref()).await?;
            let (page, next) = parse_issues_response(&body)?;
            issues.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(issues)
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

    async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction {
        let ids_body = self.fetch_assign_ids(repo, issue_number).await?;
        let ids = parse_assign_ids(&ids_body, issue_number)?;
        let mutation_body = self
            .run_assign_mutation(&ids.assignable_id, &ids.assignee_id)
            .await?;
        parse_assign_mutation(&mutation_body)
    }

    async fn add_label(&self, repo: &RepoRef, issue_number: u64, label: &str) -> AppAction {
        let ids_body = self.fetch_label_ids(repo, issue_number, label).await?;
        let ids = parse_label_ids(&ids_body, issue_number, label)?;
        let mutation_body = self
            .run_add_label_mutation(&ids.labelable_id, &ids.label_id)
            .await?;
        parse_add_label_mutation(&mutation_body)
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

/// Parse a GraphQL response body into a page of [`IssueData`]s and the cursor of
/// the next page (if any). Pure and offline: the primary test seam. Prose
/// references are resolved later by [`resolve_board`], once every page is in.
pub fn parse_response(body: &str) -> AppResult<(Vec<IssueData>, Option<String>)> {
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

/// Parse a GraphQL issues response into a page of [`RawIssue`]s and the cursor
/// of the next page (if any), for the two-tier classifier. Pure and offline:
/// the test seam for `load_issues`. Unlike [`parse_response`], every issue maps
/// directly to a [`RawIssue`] (open/closed, labels, native links, linked-PR
/// state); the cross-issue prose resolution stays in `classify_board`.
pub fn parse_issues_response(body: &str) -> AppResult<(Vec<RawIssue>, Option<String>)> {
    let response: IssuesResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_issues_response")
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
        let error = if not_found {
            AppError::not_found("Repository not found or not visible to the token")
        } else if message.to_lowercase().contains("rate limit") {
            AppError::rate_limited("GitHub rate limit exceeded")
        } else {
            AppError::internal("GitHub reported a query error")
        };
        return Err(error
            .with_operation("parse_issues_response")
            .with_context("errors", message));
    }

    let repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| {
            AppError::not_found("Repository not found or not visible to the token")
                .with_operation("parse_issues_response")
        })?;

    let issues = repository.issues;
    let raw = issues.nodes.into_iter().map(map_issue_raw).collect();
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

/// The resolved node IDs the assign-self mutation needs.
#[cfg_attr(test, derive(Debug))]
struct AssignIds {
    assignable_id: String,
    assignee_id: String,
}

/// Parse the assign-ids query response into the viewer and issue node IDs.
/// Pure and offline, so the HTTP boundary stays thin and testable. A missing
/// issue (e.g. wrong number, or not visible to the token) maps to `NotFound`.
fn parse_assign_ids(body: &str, issue_number: u64) -> AppResult<AssignIds> {
    let response: AssignIdsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_assign_ids")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(assign_error(errors, "parse_assign_ids"));
    }

    let data = response.data.ok_or_else(|| {
        AppError::internal("GitHub returned no assign data").with_operation("parse_assign_ids")
    })?;

    let issue = data
        .repository
        .and_then(|repository| repository.issue)
        .ok_or_else(|| {
            AppError::not_found("Issue not found or not visible to the token")
                .with_operation("parse_assign_ids")
                .with_context("issue", issue_number)
        })?;

    Ok(AssignIds {
        assignable_id: issue.id,
        assignee_id: data.viewer.id,
    })
}

/// Check the assign mutation response for GraphQL errors. The mutation has no
/// payload the board needs, so success is simply the absence of errors.
fn parse_assign_mutation(body: &str) -> AppAction {
    let response: MutationResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_assign_mutation")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(assign_error(errors, "parse_assign_mutation"));
    }

    Ok(())
}

/// Map a GraphQL `errors` array from an assign round trip to an [`AppError`]
/// the caller can act on, joining the messages for context.
///
/// A `FORBIDDEN` here almost always means the fine-grained token can *read*
/// issues (the board loaded) but lacks *write* access to assign them, so the
/// message names the exact permission to grant. GitHub's own text is kept out
/// of the user-facing message (it can carry backend detail) and attached as
/// diagnostic `errors` context instead.
fn assign_error(errors: Vec<GraphQlError>, operation: &'static str) -> AppError {
    let forbidden = errors.iter().any(|error| {
        matches!(error.error_type.as_deref(), Some("FORBIDDEN"))
            || error.message.to_lowercase().contains("must have")
            || error
                .message
                .to_lowercase()
                .contains("not accessible by personal access token")
    });
    let message = errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("; ");
    let lowered = message.to_lowercase();
    let error = if forbidden {
        AppError::forbidden(
            "GitHub denied the assignment. Your fine-grained token needs the \
             repository \"Issues\" permission set to \"Read and write\" (or \
             \"Pull requests: Read and write\" if the Slice is a pull request).",
        )
    } else if lowered.contains("rate limit") {
        AppError::rate_limited("GitHub rate limit exceeded")
    } else if lowered.contains("could not resolve") || lowered.contains("not_found") {
        AppError::not_found("Issue not found or not visible to the token")
    } else {
        AppError::internal("GitHub reported a query error")
    };
    error
        .with_operation(operation)
        .with_context("errors", message)
}

/// The resolved node IDs the add-label mutation needs.
#[cfg_attr(test, derive(Debug))]
struct LabelIds {
    labelable_id: String,
    label_id: String,
}

/// Parse the label-ids query response into the issue and label node IDs. Pure
/// and offline, so the HTTP boundary stays thin and testable. A missing issue
/// maps to `NotFound`; a label the repository does not define (`label: null`)
/// maps to `NotFound` naming the label, so the user can create it (the planning
/// skills do not emit `prd`/`slice` labels yet).
fn parse_label_ids(body: &str, issue_number: u64, label: &str) -> AppResult<LabelIds> {
    let response: LabelIdsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_label_ids")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(label_error(errors, "parse_label_ids"));
    }

    let repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| {
            AppError::not_found("Repository not found or not visible to the token")
                .with_operation("parse_label_ids")
        })?;

    let labelable_id = repository.issue.map(|issue| issue.id).ok_or_else(|| {
        AppError::not_found("Issue not found or not visible to the token")
            .with_operation("parse_label_ids")
            .with_context("issue", issue_number)
    })?;

    let label_id = repository.label.map(|node| node.id).ok_or_else(|| {
        AppError::not_found(format!(
            "The \"{label}\" label does not exist in this repository. Create it on \
             GitHub, then confirm the classification again."
        ))
        .with_operation("parse_label_ids")
        .with_context("label", label)
    })?;

    Ok(LabelIds {
        labelable_id,
        label_id,
    })
}

/// Check the add-label mutation response for GraphQL errors. The mutation has no
/// payload the board needs, so success is simply the absence of errors.
fn parse_add_label_mutation(body: &str) -> AppAction {
    let response: MutationResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_add_label_mutation")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(label_error(errors, "parse_add_label_mutation"));
    }

    Ok(())
}

/// Map a GraphQL `errors` array from an add-label round trip to an [`AppError`]
/// the caller can act on, joining the messages for context.
///
/// A `FORBIDDEN` here almost always means the fine-grained token can *read*
/// issues (the board loaded) but lacks *write* access to label them, so the
/// message names the exact permission to grant. GitHub's own text is kept out
/// of the user-facing message and attached as diagnostic `errors` context.
fn label_error(errors: Vec<GraphQlError>, operation: &'static str) -> AppError {
    let forbidden = errors.iter().any(|error| {
        matches!(error.error_type.as_deref(), Some("FORBIDDEN"))
            || error.message.to_lowercase().contains("must have")
            || error
                .message
                .to_lowercase()
                .contains("not accessible by personal access token")
    });
    let message = errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("; ");
    let lowered = message.to_lowercase();
    let error = if forbidden {
        AppError::forbidden(
            "GitHub denied adding the label. Your fine-grained token needs the \
             repository \"Issues\" permission set to \"Read and write\".",
        )
    } else if lowered.contains("rate limit") {
        AppError::rate_limited("GitHub rate limit exceeded")
    } else if lowered.contains("could not resolve") || lowered.contains("not_found") {
        AppError::not_found("Issue or label not found, or not visible to the token")
    } else {
        AppError::internal("GitHub reported a query error")
    };
    error
        .with_operation(operation)
        .with_context("errors", message)
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
/// that issue's identity (number, title, url) for the PRD lane, and a prose
/// blocker counts toward Blocked only if it is still open (open issues are the
/// only ones in the fetched set).
pub fn resolve_board(issues: Vec<IssueData>) -> Vec<RawSlice> {
    let open_numbers: HashSet<u64> = issues.iter().map(|issue| issue.number).collect();
    let prd_by_number: HashMap<u64, PrdRef> = issues
        .iter()
        .map(|issue| {
            (
                issue.number,
                PrdRef {
                    number: issue.number,
                    title: issue.title.clone(),
                    url: issue.url.clone(),
                },
            )
        })
        .collect();

    let mut raws: Vec<RawSlice> = issues
        .into_iter()
        .map(|issue| resolve_issue(issue, &open_numbers, &prd_by_number))
        .collect();
    // The reverse "unblocks" edge is derived across the whole board once every
    // Slice's blockers are resolved.
    domain::resolve_unblocks(&mut raws);
    raws
}

/// Project one [`IssueData`] into a [`RawSlice`], preferring native links and
/// falling back to its prose references resolved against the fetched board.
fn resolve_issue(
    issue: IssueData,
    open_numbers: &HashSet<u64>,
    prd_by_number: &HashMap<u64, PrdRef>,
) -> RawSlice {
    let prd = match issue.native_parent {
        Some(parent) => Some(parent),
        None => issue
            .prose
            .parent
            .and_then(|number| prd_by_number.get(&number).cloned()),
    };

    let open_blocker_refs = if issue.native_blockers.is_empty() {
        // No native blockers: fall back to the parsed prose, resolving each
        // still-open blocker number to its ref (with title) against the board.
        issue
            .prose
            .blocked_by
            .iter()
            .filter(|number| open_numbers.contains(number))
            .filter_map(|number| {
                prd_by_number.get(number).map(|prd| DependencyRef {
                    number: *number,
                    title: prd.title.clone(),
                    url: prd.url.clone(),
                })
            })
            .collect()
    } else {
        // Native links win: keep only the still-open blockers, with their refs.
        issue
            .native_blockers
            .iter()
            .filter(|blocker| blocker.state.as_str() == "OPEN")
            .map(|blocker| DependencyRef {
                number: blocker.number,
                title: blocker.title.clone(),
                url: blocker.url.clone(),
            })
            .collect()
    };

    RawSlice {
        number: issue.number,
        title: issue.title,
        url: issue.url,
        closed: false,
        prd,
        assignee: issue.assignee,
        has_open_linked_pr: issue.has_open_linked_pr,
        blockers: open_blocker_refs,
        // Filled by `resolve_unblocks` once the whole board is mapped.
        unblocks: Vec::new(),
    }
}

/// Project one GraphQL issue node into a [`IssueData`]: its native facts plus the
/// prose relationships parsed from its body, to be resolved by [`resolve_board`].
/// Only open issues are queried, so a closed issue never reaches this mapping.
fn map_issue(node: IssueNode) -> IssueData {
    let has_open_linked_pr = node
        .closed_by_pull_requests_references
        .nodes
        .iter()
        .any(|pr| pr.state == "OPEN");

    IssueData {
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
        native_parent: node.parent.map(|parent| PrdRef {
            number: parent.number,
            title: parent.title,
            url: parent.url,
        }),
        native_blockers: node.blocked_by.nodes,
        prose: parse_prose(&node.body),
    }
}

/// Project one GraphQL issue node into a [`RawIssue`] for the two-tier
/// classifier: open/closed, labels, native parent number (and whether it is a
/// `prd`-labelled parent), still-open native blockers, assignee, and linked-PR
/// state. The cross-issue prose resolution is left to `classify_board`.
fn map_issue_raw(node: RawIssueNode) -> RawIssue {
    let has_open_linked_pr = node
        .closed_by_pull_requests_references
        .nodes
        .iter()
        .any(|pr| pr.state == "OPEN");

    let native_parent = node.parent.as_ref().map(|parent| parent.number);
    let is_native_child_of_prd = node
        .parent
        .as_ref()
        .map(|parent| parent.labels.nodes.iter().any(|label| label.name == "prd"))
        .unwrap_or(false);

    // Only still-open blockers count toward Blocked; closed ones are dropped.
    let native_blockers = node
        .blocked_by
        .nodes
        .into_iter()
        .filter(|blocker| blocker.state == "OPEN")
        .map(|blocker| blocker.number)
        .collect();

    let body = if node.body.is_empty() {
        None
    } else {
        Some(node.body)
    };

    RawIssue {
        number: node.number,
        title: node.title,
        url: node.url,
        body,
        labels: node
            .labels
            .nodes
            .into_iter()
            .map(|label| label.name)
            .collect(),
        closed: node.state != "OPEN",
        native_parent,
        native_blockers,
        assignee: node
            .assignees
            .nodes
            .into_iter()
            .next()
            .map(|user| user.login),
        has_open_linked_pr,
        is_native_child_of_prd,
    }
}

/// A single issue's native facts plus the prose relationships parsed from its
/// body, before references are resolved against the rest of the board.
#[derive(Debug)]
pub struct IssueData {
    number: u64,
    title: String,
    url: String,
    assignee: Option<String>,
    has_open_linked_pr: bool,
    native_parent: Option<PrdRef>,
    native_blockers: Vec<BoardBlocker>,
    prose: ProseLinks,
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
struct AssignIdsResponse {
    data: Option<AssignIdsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct AssignIdsData {
    viewer: NodeId,
    repository: Option<AssignRepository>,
}

#[derive(Deserialize)]
struct AssignRepository {
    issue: Option<NodeId>,
}

#[derive(Deserialize)]
struct LabelIdsResponse {
    data: Option<LabelIdsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct LabelIdsData {
    repository: Option<LabelRepository>,
}

#[derive(Deserialize)]
struct LabelRepository {
    issue: Option<NodeId>,
    label: Option<NodeId>,
}

#[derive(Deserialize)]
struct NodeId {
    id: String,
}

#[derive(Deserialize)]
struct MutationResponse {
    errors: Option<Vec<GraphQlError>>,
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
    blocked_by: BoardBlockerConnection,
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
    number: u64,
    title: String,
    url: String,
}

#[derive(Deserialize)]
struct StateConnection {
    nodes: Vec<StateNode>,
}

#[derive(Deserialize)]
struct StateNode {
    state: String,
}

/// A native `blockedBy` dependency on the board query, with the reference the
/// blocker badges link to plus the state used to drop closed blockers.
#[derive(Deserialize)]
struct BoardBlockerConnection {
    nodes: Vec<BoardBlocker>,
}

#[derive(Debug, Deserialize)]
struct BoardBlocker {
    number: u64,
    title: String,
    url: String,
    state: String,
}

// ── Issues-for-classification query (open and closed) ────────────────────────

#[derive(Deserialize)]
struct IssuesResponse {
    data: Option<IssuesData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct IssuesData {
    repository: Option<IssuesRepositoryData>,
}

#[derive(Deserialize)]
struct IssuesRepositoryData {
    issues: RawIssueConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssueConnection {
    page_info: PageInfo,
    nodes: Vec<RawIssueNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssueNode {
    number: u64,
    title: String,
    url: String,
    body: String,
    state: String,
    labels: NameConnection,
    assignees: LoginConnection,
    parent: Option<ParentIssueNode>,
    blocked_by: BlockerConnection,
    closed_by_pull_requests_references: StateConnection,
}

#[derive(Deserialize)]
struct NameConnection {
    nodes: Vec<NameNode>,
}

#[derive(Deserialize)]
struct NameNode {
    name: String,
}

/// The native sub-issue parent, with its number and labels so the classifier
/// can tell whether it is a `prd`-labelled parent.
#[derive(Deserialize)]
struct ParentIssueNode {
    number: u64,
    labels: NameConnection,
}

#[derive(Deserialize)]
struct BlockerConnection {
    nodes: Vec<BlockerNode>,
}

#[derive(Deserialize)]
struct BlockerNode {
    number: u64,
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

#[cfg(test)]
mod tests {
    //! Offline tests for the assign-self response/error parsers, pinned against
    //! recorded GraphQL bodies so a GitHub schema or message change can't break
    //! assign-self without a test catching it.
    use super::*;
    use domain::AppErrorKind;

    #[test]
    fn parse_assign_ids_extracts_viewer_and_issue_ids() {
        let body = r#"{
            "data": {
                "viewer": { "id": "VIEWER_1" },
                "repository": { "issue": { "id": "ISSUE_42" } }
            }
        }"#;

        let ids = parse_assign_ids(body, 42).expect("ids should parse");

        assert_eq!(ids.assignee_id, "VIEWER_1");
        assert_eq!(ids.assignable_id, "ISSUE_42");
    }

    #[test]
    fn parse_assign_ids_missing_issue_is_not_found_with_context() {
        let body = r#"{
            "data": { "viewer": { "id": "VIEWER_1" }, "repository": { "issue": null } }
        }"#;

        let error = parse_assign_ids(body, 7).expect_err("a missing issue should fail");

        assert_eq!(error.kind(), AppErrorKind::NotFound);
        assert!(
            format!("{error:?}").contains("issue=7"),
            "the issue number should be attached as context: {error:?}"
        );
    }

    #[test]
    fn parse_assign_mutation_succeeds_when_no_errors() {
        let body = r#"{ "data": { "addAssigneesToAssignable": { "clientMutationId": null } } }"#;

        parse_assign_mutation(body).expect("a clean mutation response should be Ok");
    }

    #[test]
    fn assign_error_forbidden_is_actionable_and_hides_github_text() {
        let body = r#"{
            "data": null,
            "errors": [
                { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" }
            ]
        }"#;

        let error = parse_assign_mutation(body).expect_err("a FORBIDDEN response should fail");

        assert_eq!(error.kind(), AppErrorKind::Forbidden);
        // The user-facing message names the permission to grant...
        let display = error.to_string();
        assert!(
            display.contains("Read and write"),
            "the message should name the permission to grant: {display}"
        );
        // ...but never leaks GitHub's raw backend text into the UI message.
        assert!(
            !display.contains("personal access token"),
            "GitHub's raw text must not reach the user-facing message: {display}"
        );
        // The raw text is kept for diagnostics in the error context instead.
        assert!(
            format!("{error:?}").contains("personal access token"),
            "GitHub's raw text should be attached as diagnostic context: {error:?}"
        );
    }

    #[test]
    fn assign_error_maps_rate_limit_and_resolution_failures() {
        let rate_limited = r#"{ "errors": [ { "message": "API rate limit exceeded" } ] }"#;
        assert_eq!(
            parse_assign_mutation(rate_limited).unwrap_err().kind(),
            AppErrorKind::RateLimited
        );

        let unresolved =
            r#"{ "errors": [ { "message": "Could not resolve to a node with the global id" } ] }"#;
        assert_eq!(
            parse_assign_mutation(unresolved).unwrap_err().kind(),
            AppErrorKind::NotFound
        );
    }

    #[test]
    fn parse_label_ids_extracts_issue_and_label_ids() {
        let body = r#"{
            "data": {
                "repository": {
                    "issue": { "id": "ISSUE_42" },
                    "label": { "id": "LABEL_SLICE" }
                }
            }
        }"#;

        let ids = parse_label_ids(body, 42, "slice").expect("ids should parse");

        assert_eq!(ids.labelable_id, "ISSUE_42");
        assert_eq!(ids.label_id, "LABEL_SLICE");
    }

    #[test]
    fn parse_label_ids_missing_issue_is_not_found_with_context() {
        let body = r#"{
            "data": { "repository": { "issue": null, "label": { "id": "LABEL_PRD" } } }
        }"#;

        let error = parse_label_ids(body, 7, "prd").expect_err("a missing issue should fail");

        assert_eq!(error.kind(), AppErrorKind::NotFound);
        assert!(
            format!("{error:?}").contains("issue=7"),
            "the issue number should be attached as context: {error:?}"
        );
    }

    #[test]
    fn parse_label_ids_missing_label_names_the_label_to_create() {
        let body = r#"{
            "data": { "repository": { "issue": { "id": "ISSUE_5" }, "label": null } }
        }"#;

        let error = parse_label_ids(body, 5, "prd").expect_err("a missing label should fail");

        assert_eq!(error.kind(), AppErrorKind::NotFound);
        // The message names the missing label so the user knows what to create.
        let display = error.to_string();
        assert!(
            display.contains("\"prd\" label does not exist"),
            "the message should name the missing label: {display}"
        );
        assert!(
            format!("{error:?}").contains("label=prd"),
            "the label should be attached as context: {error:?}"
        );
    }

    #[test]
    fn parse_add_label_mutation_succeeds_when_no_errors() {
        let body = r#"{ "data": { "addLabelsToLabelable": { "clientMutationId": null } } }"#;

        parse_add_label_mutation(body).expect("a clean mutation response should be Ok");
    }

    #[test]
    fn label_error_forbidden_is_actionable_and_hides_github_text() {
        let body = r#"{
            "data": null,
            "errors": [
                { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" }
            ]
        }"#;

        let error = parse_add_label_mutation(body).expect_err("a FORBIDDEN response should fail");

        assert_eq!(error.kind(), AppErrorKind::Forbidden);
        // The user-facing message names the permission to grant...
        let display = error.to_string();
        assert!(
            display.contains("Read and write"),
            "the message should name the permission to grant: {display}"
        );
        // ...but never leaks GitHub's raw backend text into the UI message.
        assert!(
            !display.contains("personal access token"),
            "GitHub's raw text must not reach the user-facing message: {display}"
        );
        // The raw text is kept for diagnostics in the error context instead.
        assert!(
            format!("{error:?}").contains("personal access token"),
            "GitHub's raw text should be attached as diagnostic context: {error:?}"
        );
    }
}
