//! The real [`GitHubPort`] adapter: one GraphQL query per board-classification load.

use application::GitHubPort;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    AppAction, AppError, AppResult, LinkedPrRef, PrStatus, Project, RawIssue, RawLinkedPr, RepoRef,
    ReviewDecision, Viewer,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// One page of issues for classification: **open issues only**, with the labels
/// and native relationships the two-tier classifier needs.
///
/// It deliberately requests **no Pull Request data**. Linked PRs come from the
/// open-PR sweep ([`OPEN_PRS_QUERY`]) instead, which is what makes this query
/// shallow (a full board load dropped from a measured 46 GraphQL points to
/// about 7) and removes the four-deep nesting that once blew GitHub's node
/// limit (#168). See ADR 0008.
const ISSUES_QUERY: &str = r#"
query Issues($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, after: $cursor, states: [OPEN], orderBy: {field: CREATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        title
        url
        body
        state
        labels(first: 20) { nodes { name } }
        assignees(first: 1) { nodes { login avatarUrl } }
        parent { number labels(first: 20) { nodes { name } } }
        blockedBy(first: 50) { nodes { number } }
      }
    }
  }
}
"#;

/// One page of issue deltas for incremental refresh: open and closed issues
/// updated at or after `since`. Carries no Pull Request data either — see
/// [`ISSUES_QUERY`].
const ISSUES_SINCE_QUERY: &str = r#"
query IssuesSince($owner: String!, $name: String!, $cursor: String, $since: DateTime!) {
  repository(owner: $owner, name: $name) {
    issues(
      first: 50,
      after: $cursor,
      states: [OPEN, CLOSED],
      filterBy: { since: $since },
      orderBy: {field: UPDATED_AT, direction: DESC}
    ) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        title
        url
        body
        state
        labels(first: 20) { nodes { name } }
        assignees(first: 1) { nodes { login avatarUrl } }
        parent { number labels(first: 20) { nodes { name } } }
        blockedBy(first: 50) { nodes { number } }
      }
    }
  }
}
"#;

/// The open-PR **Sweep**: every currently-open Pull Request of the repo with the
/// facts a Linked PR renders (spine, Decorations) and the issue numbers it
/// closes.
///
/// `states: [OPEN]` is what makes "a Linked PR is an *open* PR" true by
/// construction: a merged or closed PR is simply not in the answer, so nothing
/// downstream has to filter it out (the client-side filter this replaces was the
/// only thing standing between the board and a merged PR holding a Slice in WIP
/// forever — #161, #179).
///
/// The page of 100 is a node budget: `reviewThreads(first: 100)` dominates, so
/// a page costs ~12k of GitHub's 500,000 possible-node limit, and a repo with
/// more than 100 open PRs simply pages. `every_query_fits_githubs_node_limit`
/// keeps that honest.
const OPEN_PRS_QUERY: &str = r#"
query OpenPullRequests($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(first: 100, after: $cursor, states: [OPEN], orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        url
        title
        author { login }
        isDraft
        reviewDecision
        mergeable
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        reviewThreads(first: 100) { nodes { isResolved } }
        closingIssuesReferences(first: 20) { nodes { number } }
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

/// The authenticated user's identity (login, display name, avatar), for the
/// account menu.
const VIEWER_QUERY: &str = r#"
query Viewer {
  viewer { login name avatarUrl }
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

    /// Fetch a single page of open issues for classification for
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

    /// Fetch a single page of issue deltas for `repo`, updated at or after
    /// `since`, starting after `cursor`.
    async fn fetch_issues_since_page(
        &self,
        repo: &RepoRef,
        since: &DateTime<Utc>,
        cursor: Option<&str>,
    ) -> AppResult<String> {
        let body = serde_json::json!({
            "query": ISSUES_SINCE_QUERY,
            "variables": {
                "owner": repo.owner,
                "name": repo.name,
                "cursor": cursor,
                "since": since.to_rfc3339(),
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
                    .with_operation("GitHubClient::fetch_issues_since_page")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_issues_since_page",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_issues_since_page")
                .with_source(err)
        })
    }

    /// Fetch a single page of the repo's **open** pull requests, for
    /// [`OPEN_PRS_QUERY`].
    async fn fetch_open_prs_page(&self, repo: &RepoRef, cursor: Option<&str>) -> AppResult<String> {
        let body = serde_json::json!({
            "query": OPEN_PRS_QUERY,
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
                    .with_operation("GitHubClient::fetch_open_prs_page")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_open_prs_page",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_open_prs_page")
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

    /// Fetch the authenticated user's identity (no pagination: a single object).
    async fn fetch_viewer(&self) -> AppResult<String> {
        let body = serde_json::json!({ "query": VIEWER_QUERY });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_viewer")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_viewer",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_viewer")
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

    async fn sweep_open_prs(&self, repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        let mut linked_prs = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self.fetch_open_prs_page(repo, cursor.as_deref()).await?;
            let (page, next) = parse_open_prs_response(&body)?;
            linked_prs.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(linked_prs)
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

    async fn load_issues_since(
        &self,
        repo: &RepoRef,
        since: DateTime<Utc>,
    ) -> AppResult<Vec<RawIssue>> {
        let mut issues = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let body = self
                .fetch_issues_since_page(repo, &since, cursor.as_deref())
                .await?;
            let (page, next) = parse_issues_response(&body)?;
            issues.extend(page);
            match next {
                Some(end) => cursor = Some(end),
                None => break,
            }
        }

        Ok(issues)
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

    async fn viewer(&self) -> AppResult<Viewer> {
        let body = self.fetch_viewer().await?;
        parse_viewer_response(&body)
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

/// Parse a GraphQL issues response into a page of [`RawIssue`]s and the cursor
/// of the next page (if any), for the two-tier classifier. Pure and offline:
/// the test seam for `load_issues`. Every issue maps
/// directly to a [`RawIssue`] (open/closed, labels, native links, linked-PR
/// state); the cross-issue prose resolution stays in `classify_board`.
pub fn parse_issues_response(body: &str) -> AppResult<(Vec<RawIssue>, Option<String>)> {
    let response: IssuesResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_issues_response")
            .with_source(err)
    })?;

    // GitHub returns partial data alongside field-level errors — e.g. a
    // FORBIDDEN on `statusCheckRollup` when the token cannot read a repo's CI
    // checks. When the repository data is present, use it (a forbidden optional
    // field simply comes back null) rather than failing the whole board on a
    // partial error. Only when there is no usable repository do the errors
    // become fatal.
    if let Some(repository) = response.data.and_then(|data| data.repository) {
        let issues = repository.issues;
        let raw = issues.nodes.into_iter().map(map_issue_raw).collect();
        let next = if issues.page_info.has_next_page {
            issues.page_info.end_cursor
        } else {
            None
        };
        return Ok((raw, next));
    }

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(repository_query_error(errors, "parse_issues_response"));
    }

    Err(
        AppError::not_found("Repository not found or not visible to the token")
            .with_operation("parse_issues_response"),
    )
}

/// Map a GraphQL `errors` array from a repository read (issues or the open-PR
/// sweep) to an [`AppError`] the caller can act on, joining the messages as
/// diagnostic context.
///
/// A whole-repository `FORBIDDEN` (not just an optional field) — e.g. a
/// fine-grained token with no Issues or Pull requests permission at all — must
/// classify as Forbidden, not Internal, so `CredentialFailure::UnderScoped`
/// routes to Rotate instead of a generic error.
fn repository_query_error(errors: Vec<GraphQlError>, operation: &'static str) -> AppError {
    let not_found = errors.iter().any(|error| {
        error.error_type.as_deref() == Some("NOT_FOUND")
            || error
                .message
                .to_lowercase()
                .contains("could not resolve to a repository")
    });
    let forbidden = errors
        .iter()
        .any(|error| error.error_type.as_deref() == Some("FORBIDDEN"));
    let message = errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("; ");
    let error = if not_found {
        AppError::not_found("Repository not found or not visible to the token")
    } else if forbidden {
        AppError::forbidden("The token lacks access to this repository")
    } else if message.to_lowercase().contains("rate limit") {
        AppError::rate_limited("GitHub rate limit exceeded")
    } else {
        AppError::internal("GitHub reported a query error")
    };
    error
        .with_operation(operation)
        .with_context("errors", message)
}

/// Parse a page of [`OPEN_PRS_QUERY`] into the swept [`RawLinkedPr`]s and the
/// cursor of the next page (if any). Pure and offline, mirroring
/// [`parse_issues_response`] — including its tolerance of field-level errors
/// beside the data (e.g. a token that cannot read a repo's CI checks).
pub fn parse_open_prs_response(body: &str) -> AppResult<(Vec<RawLinkedPr>, Option<String>)> {
    let response: OpenPrsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_open_prs_response")
            .with_source(err)
    })?;

    if let Some(pull_requests) = response
        .data
        .and_then(|data| data.repository)
        .map(|repository| repository.pull_requests)
    {
        let linked_prs = pull_requests.nodes.into_iter().map(map_linked_pr).collect();
        let next = if pull_requests.page_info.has_next_page {
            pull_requests.page_info.end_cursor
        } else {
            None
        };
        return Ok((linked_prs, next));
    }

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(repository_query_error(errors, "parse_open_prs_response"));
    }

    Err(
        AppError::not_found("Repository not found or not visible to the token")
            .with_operation("parse_open_prs_response"),
    )
}

/// Parse a GraphQL projects response into a page of [`Project`]s and the cursor
/// of the next page (if any). Pure and offline.
pub fn parse_projects_response(body: &str) -> AppResult<(Vec<Project>, Option<String>)> {
    let response: ProjectsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_projects_response")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        let forbidden = errors
            .iter()
            .any(|error| error.error_type.as_deref() == Some("FORBIDDEN"));
        let message = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        let error = if forbidden {
            // The token cannot list repositories at all (e.g. missing the
            // Metadata/Contents permission). Must classify Forbidden, not
            // Internal, so CredentialFailure::UnderScoped routes to Rotate
            // instead of a generic error.
            AppError::forbidden("The token lacks access to your repositories")
        } else if message.to_lowercase().contains("rate limit") {
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

/// Parse a GraphQL viewer response into the domain [`Viewer`]. Pure and
/// offline, so the HTTP boundary stays thin and testable.
pub fn parse_viewer_response(body: &str) -> AppResult<Viewer> {
    let response: ViewerResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_viewer_response")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(viewer_error(errors, "parse_viewer_response"));
    }

    let node = response.data.map(|data| data.viewer).ok_or_else(|| {
        AppError::internal("GitHub returned no viewer data").with_operation("parse_viewer_response")
    })?;

    Ok(Viewer {
        login: node.login,
        name: node.name,
        avatar_url: node.avatar_url,
    })
}

/// Map a GraphQL `errors` array from a viewer round trip to an [`AppError`] the
/// caller can act on, joining the messages for context.
///
/// A `FORBIDDEN` here means the token cannot read even its own basic profile
/// (e.g. a token GitHub has otherwise rejected or under-scoped), so it is
/// classified the same way `assign_error`/`label_error` classify a missing
/// grant — as `Forbidden`, not `Internal` — so callers can route it through the
/// same Under-scoped handling as every other GitHub operation.
fn viewer_error(errors: Vec<GraphQlError>, operation: &'static str) -> AppError {
    let forbidden = errors.iter().any(|error| {
        matches!(error.error_type.as_deref(), Some("FORBIDDEN"))
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
            "GitHub denied reading your profile. Your fine-grained token may be \
             missing a required permission.",
        )
    } else if lowered.contains("rate limit") {
        AppError::rate_limited("GitHub rate limit exceeded")
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

/// Map GitHub's `reviewDecision` string to the domain [`ReviewDecision`]. A null
/// decision (review not required, or none reached) or any unrecognised value
/// maps to `None`, which [`PrStatus::derive`] reads as "awaiting review".
fn review_decision(raw: Option<&str>) -> Option<ReviewDecision> {
    match raw {
        Some("APPROVED") => Some(ReviewDecision::Approved),
        Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
        Some("REVIEW_REQUIRED") => Some(ReviewDecision::ReviewRequired),
        _ => None,
    }
}

/// Project one swept pull request node into a [`RawLinkedPr`]: the facts a
/// `pr #n @u` badge renders (spine + Decorations) plus the issue numbers it
/// closes.
///
/// No open/closed filtering happens here, and none is needed: the sweep asks
/// for `states: [OPEN]`, so a merged or closed PR never reaches this function.
/// A null `author` (e.g. a deleted account) leaves the `@u` segment off the
/// badge.
fn map_linked_pr(node: OpenPrNode) -> RawLinkedPr {
    RawLinkedPr {
        pr: LinkedPrRef {
            number: node.number,
            author: node.author.map(|author| author.login),
            title: node.title,
            url: node.url,
            pr_status: PrStatus::derive(
                node.is_draft,
                review_decision(node.review_decision.as_deref()),
            ),
            conflicts: node.mergeable.as_deref() == Some("CONFLICTING"),
            ci_failing: node
                .commits
                .nodes
                .first()
                .and_then(|commit| commit.commit.status_check_rollup.as_ref())
                .map(|rollup| matches!(rollup.state.as_str(), "FAILURE" | "ERROR"))
                .unwrap_or(false),
            unresolved_comment_count: node
                .review_threads
                .nodes
                .iter()
                .filter(|thread| !thread.is_resolved)
                .count() as u32,
        },
        closes: node
            .closing_issues_references
            .nodes
            .into_iter()
            .map(|issue| issue.number)
            .collect(),
    }
}

/// Project one GraphQL issue node into a [`RawIssue`] for the two-tier
/// classifier: open/closed, labels, native parent number (and whether it is a
/// `prd`-labelled parent), native blockers (open and closed), and assignee. No
/// Pull Request data: that comes from the open-PR sweep. The cross-issue
/// open-set filtering and prose resolution are left to `classify`.
fn map_issue_raw(node: RawIssueNode) -> RawIssue {
    let native_parent = node.parent.as_ref().map(|parent| parent.number);
    let is_native_child_of_prd = node
        .parent
        .as_ref()
        .map(|parent| parent.labels.nodes.iter().any(|label| label.name == "prd"))
        .unwrap_or(false);

    // Carry every native blocker (open and closed); classifier-level filtering
    // resolves the board's currently-open set.
    let native_blockers = node
        .blocked_by
        .nodes
        .into_iter()
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
        assignee: node.assignees.nodes.first().map(|user| user.login.clone()),
        assignee_avatar_url: node
            .assignees
            .nodes
            .first()
            .map(|user| user.avatar_url.clone()),
        is_native_child_of_prd,
    }
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
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct LoginConnection {
    nodes: Vec<Login>,
}

#[derive(Deserialize)]
struct Login {
    login: String,
    #[serde(rename = "avatarUrl")]
    avatar_url: String,
}

// ── Open-PR sweep query ───────────────────────────────────────────────

#[derive(Deserialize)]
struct OpenPrsResponse {
    data: Option<OpenPrsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct OpenPrsData {
    repository: Option<OpenPrsRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenPrsRepository {
    pull_requests: OpenPrConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenPrConnection {
    page_info: PageInfo,
    nodes: Vec<OpenPrNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenPrNode {
    number: u64,
    url: String,
    title: String,
    author: Option<AuthorNode>,
    #[serde(default)]
    is_draft: bool,
    review_decision: Option<String>,
    mergeable: Option<String>,
    #[serde(default)]
    commits: CommitConnection,
    #[serde(default)]
    review_threads: ReviewThreadConnection,
    #[serde(default)]
    closing_issues_references: ClosingIssuesConnection,
}

/// The issue numbers a pull request closes (its closing references) — the edge
/// the board joins Linked PRs onto Slices by.
#[derive(Deserialize, Default)]
struct ClosingIssuesConnection {
    nodes: Vec<ClosingIssueNode>,
}

#[derive(Deserialize)]
struct ClosingIssueNode {
    number: u64,
}

/// The PR's last commit (via `commits(last: 1)`), carrying the aggregated CI
/// check rollup used for the CI-failing Decoration.
#[derive(Deserialize, Default)]
struct CommitConnection {
    nodes: Vec<PullRequestCommitNode>,
}

#[derive(Deserialize)]
struct PullRequestCommitNode {
    commit: CommitNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitNode {
    status_check_rollup: Option<StatusCheckRollup>,
}

#[derive(Deserialize)]
struct StatusCheckRollup {
    state: String,
}

/// The PR's review threads, for counting the unresolved ones (the
/// Unresolved-comments Decoration).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadConnection {
    nodes: Vec<ReviewThreadNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadNode {
    is_resolved: bool,
}

#[derive(Deserialize)]
struct AuthorNode {
    login: String,
}
// ── Issues-for-classification query (open only) ──────────────────────────────

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
}

#[derive(Deserialize)]
struct ProjectsResponse {
    data: Option<ProjectsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct ProjectsData {
    viewer: ProjectsViewer,
}

#[derive(Deserialize)]
struct ProjectsViewer {
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

#[derive(Deserialize)]
struct ViewerResponse {
    data: Option<ViewerResponseData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct ViewerResponseData {
    viewer: ViewerNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewerNode {
    login: String,
    name: Option<String>,
    avatar_url: String,
}

#[cfg(test)]
mod tests {
    //! Offline tests for the GraphQL response/error parsers (assign-self,
    //! add-label), pinned against recorded
    //! GraphQL bodies so a GitHub schema or message change can't break a parser
    //! without a test catching it.
    use super::*;
    use domain::AppErrorKind;

    /// GitHub's static cap on a single query: it charges the **possible** node
    /// count — the product of the page sizes along each path through the
    /// selection set, summed over every connection — and rejects the query with
    /// `MAX_NODE_LIMIT_EXCEEDED` before reading a single row.
    const GITHUB_NODE_LIMIT: u64 = 500_000;

    /// The worst-case node count GitHub charges `query`, computed the way GitHub
    /// does: every connection (a field carrying a `first:`/`last:` page size)
    /// contributes `enclosing page sizes x its own page size`, and the
    /// contributions are summed.
    ///
    /// Reading the query text rather than restating its page sizes is what makes
    /// this a guard: a field added to a shared selection is priced automatically.
    fn worst_case_nodes(query: &str) -> u64 {
        let mut total = 0;
        // Multiplier of the selection set currently open; the outermost one
        // (the query root) returns a single object.
        let mut multipliers = vec![1_u64];
        // Page size of the field whose selection set is about to open, if it is
        // a connection.
        let mut pending: Option<u64> = None;
        let mut rest = query;

        while let Some(index) = rest.find(['(', '{', '}']) {
            let after = &rest[index..];
            match after.as_bytes()[0] {
                // An argument list. Skipped whole, so braces *inside* it
                // (`orderBy: {…}`, `filterBy: {…}`) are never mistaken for a
                // selection set.
                b'(' => {
                    let end = after.find(')').map_or(after.len(), |end| end + 1);
                    pending = page_size(&after[..end]);
                    rest = &after[end..];
                }
                b'{' => {
                    let enclosing = *multipliers.last().expect("the root is never popped");
                    // A plain object field (`nodes`, `parent`, `commit`) has no
                    // page size: it neither multiplies nor is charged.
                    let multiplier = enclosing * pending.unwrap_or(1);
                    if pending.take().is_some() {
                        total += multiplier;
                    }
                    multipliers.push(multiplier);
                    rest = &after[1..];
                }
                _ => {
                    multipliers.pop();
                    rest = &after[1..];
                }
            }
        }

        total
    }

    /// The `first:`/`last:` page size in a GraphQL argument list, if it has one.
    fn page_size(arguments: &str) -> Option<u64> {
        ["first:", "last:"].iter().find_map(|key| {
            let start = arguments.find(key)? + key.len();
            let digits: String = arguments[start..]
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
    }

    #[test]
    fn the_node_estimator_prices_nesting_the_way_github_does() {
        // 10 issues, each with 5 labels: 10 + 10 x 5. `nodes` and `parent` are
        // plain object fields and cost nothing; the `orderBy` braces sit inside
        // an argument list and are not a selection set.
        let query = r#"
            query Sample($cursor: String) {
              repository(owner: "acme", name: "widgets") {
                issues(first: 10, after: $cursor, orderBy: {field: CREATED_AT, direction: ASC}) {
                  nodes {
                    number
                    parent { number }
                    labels(first: 5) { nodes { name } }
                  }
                }
              }
            }
        "#;

        assert_eq!(worst_case_nodes(query), 60);
    }

    #[test]
    fn every_query_fits_githubs_node_limit() {
        // A query over the limit is rejected outright, so this is not a
        // performance preference: the four-deep PR query this design replaced
        // shipped at 1,000,000 possible nodes and *never once* succeeded against
        // real GitHub, because the parse tests replay recorded bodies (#168).
        for (name, query) in [
            ("ISSUES_QUERY", ISSUES_QUERY),
            ("ISSUES_SINCE_QUERY", ISSUES_SINCE_QUERY),
            ("OPEN_PRS_QUERY", OPEN_PRS_QUERY),
            ("PROJECTS_QUERY", PROJECTS_QUERY),
        ] {
            let nodes = worst_case_nodes(query);
            assert!(
                nodes <= GITHUB_NODE_LIMIT,
                "{name} requests up to {nodes} possible nodes, over GitHub's limit of {GITHUB_NODE_LIMIT}"
            );
        }
    }

    #[test]
    fn no_issue_side_query_requests_pull_request_data() {
        // Linked PRs are read from the PR side by the sweep (ADR 0008). An issue
        // query that reached back into pull requests would reintroduce both the
        // node-limit blowout (#168) and the merged-PR-shown-as-open bug (#179),
        // since a nested connection also returns MERGED references.
        for (name, query) in [
            ("ISSUES_QUERY", ISSUES_QUERY),
            ("ISSUES_SINCE_QUERY", ISSUES_SINCE_QUERY),
        ] {
            let lowered = query.to_lowercase();
            assert!(
                !lowered.contains("pullrequest"),
                "{name} must request no Pull Request data"
            );
        }
    }

    #[test]
    fn issues_since_query_requests_assignee_avatar_url() {
        // Regression guard: `parse_issues_response` (shared by the full and
        // since queries) requires `avatarUrl` on every assignee. The since
        // query used to omit it from its selection set, so any assigned issue
        // in a delta page would fail to deserialize and silently error out the
        // whole background refresh.
        assert!(
            ISSUES_SINCE_QUERY.contains("login avatarUrl"),
            "ISSUES_SINCE_QUERY must request avatarUrl alongside login"
        );
    }

    #[test]
    fn parse_issues_tolerates_partial_field_errors_when_data_is_present() {
        // GitHub can return the issue data *plus* a field-level error (e.g. a
        // FORBIDDEN on a field the token cannot read). The board must still load
        // from the data rather than failing wholesale.
        let body = r#"{
            "data": { "repository": { "issues": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [ {
                    "number": 1, "title": "A Slice", "url": "https://x/1", "body": "",
                    "state": "OPEN", "labels": { "nodes": [ { "name": "slice" } ] },
                    "assignees": { "nodes": [] }, "parent": null,
                    "blockedBy": { "nodes": [] }
                } ]
            } } },
            "errors": [ { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token",
                "path": ["repository","issues","nodes",0,"assignees"] } ]
        }"#;

        let (issues, next) =
            parse_issues_response(body).expect("partial field errors must not fail the board");
        assert_eq!(next, None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 1);
    }

    #[test]
    fn parse_issues_still_fails_when_no_repository_data() {
        // A whole-repository FORBIDDEN (no data) must still surface as an error.
        let body = r#"{ "data": { "repository": null },
            "errors": [ { "type": "NOT_FOUND", "message": "Could not resolve to a repository" } ] }"#;
        let error = parse_issues_response(body).expect_err("absent repository must fail");
        assert_eq!(error.kind(), AppErrorKind::NotFound);
    }

    #[test]
    fn parse_issues_maps_a_whole_repository_forbidden_to_forbidden_not_internal() {
        // A Forbidden GraphQL error (token lacks a permission the whole query
        // needs, e.g. Issues: Read) must classify as Forbidden so the
        // CredentialFailure::UnderScoped routing actually triggers — falling
        // through to Internal would show a generic error instead of Rotate.
        let body = r#"{ "data": { "repository": null },
            "errors": [ { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" } ] }"#;
        let error = parse_issues_response(body).expect_err("a FORBIDDEN response should fail");
        assert_eq!(error.kind(), AppErrorKind::Forbidden);
    }

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

    #[test]
    fn parse_viewer_extracts_login_name_and_avatar() {
        let body = r#"{
            "data": {
                "viewer": {
                    "login": "carlos-verdes",
                    "name": "Carlos Verdes",
                    "avatarUrl": "https://avatars.githubusercontent.com/u/9919?v=4"
                }
            }
        }"#;

        let viewer = parse_viewer_response(body).expect("a clean viewer response should parse");

        assert_eq!(viewer.login, "carlos-verdes");
        assert_eq!(viewer.name.as_deref(), Some("Carlos Verdes"));
        assert_eq!(
            viewer.avatar_url,
            "https://avatars.githubusercontent.com/u/9919?v=4"
        );
    }

    #[test]
    fn parse_viewer_tolerates_a_missing_display_name() {
        let body = r#"{
            "data": {
                "viewer": {
                    "login": "carlos-verdes",
                    "name": null,
                    "avatarUrl": "https://avatars.githubusercontent.com/u/9919?v=4"
                }
            }
        }"#;

        let viewer = parse_viewer_response(body).expect("a null name should still parse");

        assert_eq!(viewer.name, None);
    }

    #[test]
    fn parse_viewer_maps_rate_limit_errors() {
        let body = r#"{ "errors": [ { "message": "API rate limit exceeded" } ] }"#;

        let error = parse_viewer_response(body).expect_err("a rate-limit error should fail");

        assert_eq!(error.kind(), AppErrorKind::RateLimited);
    }

    #[test]
    fn parse_viewer_maps_forbidden_to_forbidden_not_internal() {
        let body = r#"{
            "data": null,
            "errors": [
                { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" }
            ]
        }"#;

        let error = parse_viewer_response(body).expect_err("a FORBIDDEN response should fail");

        assert_eq!(error.kind(), AppErrorKind::Forbidden);
        // GitHub's raw text must not leak into the user-facing message...
        let display = error.to_string();
        assert!(
            !display.contains("personal access token"),
            "GitHub's raw text must not reach the user-facing message: {display}"
        );
        // ...but is kept for diagnostics in the error context instead.
        assert!(
            format!("{error:?}").contains("personal access token"),
            "GitHub's raw text should be attached as diagnostic context: {error:?}"
        );
    }

    #[test]
    fn parse_viewer_maps_other_query_errors_to_internal() {
        let body = r#"{ "errors": [ { "message": "Something went wrong" } ] }"#;

        let error = parse_viewer_response(body).expect_err("a query error should fail");

        assert_eq!(error.kind(), AppErrorKind::Internal);
    }
}
