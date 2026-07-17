//! The real [`GitHubPort`] adapter: one GraphQL query per board-classification load.

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AgentRef, AppAction, AppError, AppResult, LinkedPrRef, Project, RawIssue, RepoRef};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// One page of issues for classification: **open issues only**, with the labels,
/// native relationships, and linked-PR state the two-tier classifier needs.
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
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { number url title author { login } } }
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

/// Resolve the node IDs the delegate (assign-agent) mutation needs in one round
/// trip: the target issue (assignable), the repository (the agent session's
/// target), its default branch (the session's base ref), and the live
/// `suggestedActors` list so the chosen Agent's node ID is re-resolved at action
/// time (the board-load value may be stale or the Agent no longer assignable).
const AGENT_ASSIGN_IDS_QUERY: &str = r#"
query AgentAssignIds($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    id
    defaultBranchRef { name }
    issue(number: $number) { id }
    suggestedActors(capabilities: [CAN_BE_ASSIGNED], first: 100) {
      nodes {
        __typename
        login
        ... on Node { id }
      }
    }
  }
}
"#;

/// Delegate an issue to an Agent: extend [`ASSIGN_MUTATION`] with `agentAssignment`
/// so GitHub starts a coding session, and send the [`GRAPHQL_FEATURES_HEADER`]
/// feature flags. The board re-polls after success, so the now-assigned Slice
/// derives `Wip`.
const AGENT_ASSIGN_MUTATION: &str = r#"
mutation AssignAgent($assignableId: ID!, $assigneeId: ID!, $repositoryId: ID!, $baseRef: String!) {
  addAssigneesToAssignable(input: {
    assignableId: $assignableId,
    assigneeIds: [$assigneeId],
    agentAssignment: {targetRepositoryId: $repositoryId, baseRef: $baseRef}
  }) {
    clientMutationId
  }
}
"#;

/// The GraphQL feature flags that unlock the `agentAssignment` input and start a
/// Copilot coding session (per spike 0001). Sent only on the delegate mutation.
const GRAPHQL_FEATURES_HEADER: &str =
    "issues_copilot_assignment_api_support,coding_agent_model_selection";

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

/// Actors that can be assigned on a repository, filtered to those with the
/// `CAN_BE_ASSIGNED` capability. Only bot actors are kept; an empty result
/// (e.g. Copilot not enabled) is a valid success.
const SUGGESTED_ACTORS_QUERY: &str = r#"
query SuggestedActors($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    suggestedActors(capabilities: [CAN_BE_ASSIGNED], first: 10) {
      nodes {
        __typename
        login
        ... on Node { id }
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

    /// Resolve the issue, repository, default branch and live Agent node IDs the
    /// delegate mutation needs.
    async fn fetch_agent_assign_ids(&self, repo: &RepoRef, issue_number: u64) -> AppResult<String> {
        let body = serde_json::json!({
            "query": AGENT_ASSIGN_IDS_QUERY,
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
                    .with_operation("GitHubClient::fetch_agent_assign_ids")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_agent_assign_ids",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_agent_assign_ids")
                .with_source(err)
        })
    }

    /// Run the `addAssigneesToAssignable` delegate mutation, attaching the
    /// `agentAssignment` input via the feature-flag header so GitHub starts a
    /// coding session for the resolved node IDs.
    async fn run_agent_assign_mutation(&self, ids: &AgentAssignIds) -> AppResult<String> {
        let body = serde_json::json!({
            "query": AGENT_ASSIGN_MUTATION,
            "variables": {
                "assignableId": ids.assignable_id,
                "assigneeId": ids.assignee_id,
                "repositoryId": ids.repository_id,
                "baseRef": ids.base_ref,
            },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .header("GraphQL-Features", GRAPHQL_FEATURES_HEADER)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::run_agent_assign_mutation")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::run_agent_assign_mutation",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::run_agent_assign_mutation")
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

    /// Fetch the actors that can be assigned on `repo`, for Agent discovery.
    async fn fetch_suggested_actors(&self, repo: &RepoRef) -> AppResult<String> {
        let body = serde_json::json!({
            "query": SUGGESTED_ACTORS_QUERY,
            "variables": { "owner": repo.owner, "name": repo.name },
        });

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                AppError::unavailable("Could not reach GitHub")
                    .with_operation("GitHubClient::fetch_suggested_actors")
                    .with_source(err)
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(status_error(
                status,
                &response,
                "GitHubClient::fetch_suggested_actors",
            ));
        }

        response.text().await.map_err(|err| {
            AppError::unavailable("Could not read GitHub's response")
                .with_operation("GitHubClient::fetch_suggested_actors")
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

    async fn assign_agent(&self, repo: &RepoRef, issue_number: u64, agent: &AgentRef) -> AppAction {
        let ids_body = self.fetch_agent_assign_ids(repo, issue_number).await?;
        let ids = parse_agent_assign_ids(&ids_body, issue_number, &agent.name)?;
        let mutation_body = self.run_agent_assign_mutation(&ids).await?;
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

    async fn suggested_agents(&self, repo: &RepoRef) -> AppResult<Vec<AgentRef>> {
        let body = self.fetch_suggested_actors(repo).await?;
        parse_suggested_actors_response(&body)
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
/// of the next page (if any). Pure and offline.
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

/// The resolved node IDs and base ref the delegate (assign-agent) mutation needs.
#[cfg_attr(test, derive(Debug))]
struct AgentAssignIds {
    assignable_id: String,
    assignee_id: String,
    repository_id: String,
    base_ref: String,
}

/// Parse the agent-assign-ids query response into the IDs the delegate mutation
/// needs, re-resolving the chosen Agent's node ID live by matching `agent_name`
/// against the current `suggestedActors`. Pure and offline, so the HTTP boundary
/// stays thin and testable.
///
/// A missing issue maps to `NotFound`; an Agent that is no longer in the
/// assignable set (e.g. Copilot disabled since board load) also maps to
/// `NotFound` naming the Agent, so the board can surface a clear message and
/// leave the Slice unchanged.
fn parse_agent_assign_ids(
    body: &str,
    issue_number: u64,
    agent_name: &str,
) -> AppResult<AgentAssignIds> {
    let response: AgentAssignIdsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_agent_assign_ids")
            .with_source(err)
    })?;

    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(assign_error(errors, "parse_agent_assign_ids"));
    }

    let repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| {
            AppError::not_found("Repository not found or not visible to the token")
                .with_operation("parse_agent_assign_ids")
        })?;

    let issue = repository.issue.ok_or_else(|| {
        AppError::not_found("Issue not found or not visible to the token")
            .with_operation("parse_agent_assign_ids")
            .with_context("issue", issue_number)
    })?;

    let base_ref = repository
        .default_branch_ref
        .map(|reference| reference.name)
        .ok_or_else(|| {
            AppError::internal("The repository has no default branch to start the Agent from")
                .with_operation("parse_agent_assign_ids")
        })?;

    let assignee_id = repository
        .suggested_actors
        .nodes
        .into_iter()
        .filter(|node| node.typename == "Bot")
        .find(|node| node.login == agent_name)
        .and_then(|node| node.id)
        .ok_or_else(|| {
            AppError::not_found("That Agent is no longer assignable on this repository")
                .with_operation("parse_agent_assign_ids")
                .with_context("agent", agent_name)
        })?;

    Ok(AgentAssignIds {
        assignable_id: issue.id,
        assignee_id,
        repository_id: repository.id,
        base_ref,
    })
}

#[derive(Deserialize)]
struct AgentAssignIdsResponse {
    data: Option<AgentAssignIdsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct AgentAssignIdsData {
    repository: Option<AgentAssignRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentAssignRepository {
    id: String,
    default_branch_ref: Option<RefName>,
    issue: Option<NodeId>,
    suggested_actors: ActorConnection,
}

#[derive(Deserialize)]
struct RefName {
    name: String,
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

/// Project one GraphQL issue node into a [`RawIssue`] for the two-tier
/// classifier: open/closed, labels, native parent number (and whether it is a
/// `prd`-labelled parent), native blockers (open and closed), assignee, and
/// linked-PR state. The cross-issue open-set filtering and prose resolution are
/// left to `classify_board`.
fn map_issue_raw(node: RawIssueNode) -> RawIssue {
    // `includeClosedPrs: false` means every returned node is an open linked PR,
    // so each maps straight to a `LinkedPrRef`. A null `author` (e.g. a deleted
    // account) leaves the `@u` segment off the badge.
    let linked_prs = node
        .closed_by_pull_requests_references
        .nodes
        .into_iter()
        .map(|pr| LinkedPrRef {
            number: pr.number,
            author: pr.author.map(|author| author.login),
            title: pr.title,
            url: pr.url,
        })
        .collect();

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
        linked_prs,
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

#[derive(Deserialize)]
struct LinkedPrConnection {
    nodes: Vec<LinkedPrNode>,
}

#[derive(Deserialize)]
struct LinkedPrNode {
    number: u64,
    url: String,
    title: String,
    author: Option<AuthorNode>,
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
    closed_by_pull_requests_references: LinkedPrConnection,
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

// ── Suggested-actors query ────────────────────────────────────────────────────

/// Parse a `suggestedActors` response into a list of [`AgentRef`]s.
///
/// Only bot actors (`__typename == "Bot"`) are kept; users and organizations are
/// discarded. An empty node list (Copilot not enabled) maps to `Ok(vec![])`.
/// Pure and offline: the primary test seam for Agent discovery.
pub fn parse_suggested_actors_response(body: &str) -> AppResult<Vec<AgentRef>> {
    let response: SuggestedActorsResponse = serde_json::from_str(body).map_err(|err| {
        AppError::internal("GitHub returned a malformed response")
            .with_operation("parse_suggested_actors_response")
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
            .with_operation("parse_suggested_actors_response")
            .with_context("errors", message));
    }

    let agents = response
        .data
        .and_then(|data| data.repository)
        .map(|repo| {
            repo.suggested_actors
                .nodes
                .into_iter()
                .filter(|node| node.typename == "Bot")
                .filter_map(|node| {
                    node.id.map(|id| AgentRef {
                        name: node.login,
                        node_id: id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(agents)
}

#[derive(Deserialize)]
struct SuggestedActorsResponse {
    data: Option<SuggestedActorsData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct SuggestedActorsData {
    repository: Option<SuggestedActorsRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuggestedActorsRepository {
    suggested_actors: ActorConnection,
}

#[derive(Deserialize)]
struct ActorConnection {
    nodes: Vec<ActorNode>,
}

/// A single actor node from `suggestedActors`.
///
/// `__typename` is used to filter to bot actors only. `id` is `None` only when
/// the inline fragment `... on Node { id }` did not match — which cannot happen
/// in practice since every GitHub actor implements `Node`, but we treat it as
/// optional for resilience.
#[derive(Deserialize)]
struct ActorNode {
    #[serde(rename = "__typename")]
    typename: String,
    login: String,
    id: Option<String>,
}

#[cfg(test)]
mod tests {
    //! Offline tests for the GraphQL response/error parsers (assign-self,
    //! assign-agent, add-label, and suggested-actors), pinned against recorded
    //! GraphQL bodies so a GitHub schema or message change can't break a parser
    //! without a test catching it.
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
    fn parse_agent_assign_ids_resolves_ids_and_live_bot() {
        let body = r#"{
            "data": {
                "repository": {
                    "id": "REPO_1",
                    "defaultBranchRef": { "name": "main" },
                    "issue": { "id": "ISSUE_42" },
                    "suggestedActors": {
                        "nodes": [
                            { "__typename": "User", "login": "alice", "id": "USER_1" },
                            { "__typename": "Bot", "login": "copilot-swe-agent", "id": "BOT_LIVE" }
                        ]
                    }
                }
            }
        }"#;

        let ids = parse_agent_assign_ids(body, 42, "copilot-swe-agent").expect("ids should parse");

        assert_eq!(ids.assignable_id, "ISSUE_42");
        assert_eq!(ids.repository_id, "REPO_1");
        assert_eq!(ids.base_ref, "main");
        // The Agent's node ID is the one resolved live, not the caller's value.
        assert_eq!(ids.assignee_id, "BOT_LIVE");
    }

    #[test]
    fn parse_agent_assign_ids_unknown_agent_is_not_found_with_context() {
        let body = r#"{
            "data": {
                "repository": {
                    "id": "REPO_1",
                    "defaultBranchRef": { "name": "main" },
                    "issue": { "id": "ISSUE_42" },
                    "suggestedActors": { "nodes": [] }
                }
            }
        }"#;

        let error = parse_agent_assign_ids(body, 42, "copilot-swe-agent")
            .expect_err("an Agent absent from the assignable set should fail");

        assert_eq!(error.kind(), AppErrorKind::NotFound);
        assert!(
            format!("{error:?}").contains("agent=copilot-swe-agent"),
            "the Agent name should be attached as context: {error:?}"
        );
    }

    #[test]
    fn parse_agent_assign_ids_missing_issue_is_not_found_with_context() {
        let body = r#"{
            "data": {
                "repository": {
                    "id": "REPO_1",
                    "defaultBranchRef": { "name": "main" },
                    "issue": null,
                    "suggestedActors": {
                        "nodes": [
                            { "__typename": "Bot", "login": "copilot-swe-agent", "id": "BOT_LIVE" }
                        ]
                    }
                }
            }
        }"#;

        let error = parse_agent_assign_ids(body, 7, "copilot-swe-agent")
            .expect_err("a missing issue should fail");

        assert_eq!(error.kind(), AppErrorKind::NotFound);
        assert!(
            format!("{error:?}").contains("issue=7"),
            "the issue number should be attached as context: {error:?}"
        );
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

    // ── parse_suggested_actors_response ──────────────────────────────────────

    #[test]
    fn parse_suggested_actors_empty_nodes_returns_empty_vec() {
        let body = r#"{
            "data": {
                "repository": {
                    "suggestedActors": { "nodes": [] }
                }
            }
        }"#;

        let agents =
            parse_suggested_actors_response(body).expect("empty nodes should be Ok(vec![])");

        assert!(agents.is_empty(), "no actors → empty agent set");
    }

    #[test]
    fn parse_suggested_actors_keeps_only_bot_actors() {
        let body = r#"{
            "data": {
                "repository": {
                    "suggestedActors": {
                        "nodes": [
                            { "__typename": "Bot",  "login": "copilot",      "id": "BOT_NODE_1" },
                            { "__typename": "User", "login": "carlos-verdes","id": "USER_NODE_2" }
                        ]
                    }
                }
            }
        }"#;

        let agents = parse_suggested_actors_response(body).expect("response should parse");

        assert_eq!(agents.len(), 1, "only the Bot actor should be kept");
        assert_eq!(agents[0].name, "copilot");
        assert_eq!(agents[0].node_id, "BOT_NODE_1");
    }

    #[test]
    fn parse_suggested_actors_maps_multiple_bots() {
        let body = r#"{
            "data": {
                "repository": {
                    "suggestedActors": {
                        "nodes": [
                            { "__typename": "Bot", "login": "copilot",   "id": "BOT_1" },
                            { "__typename": "Bot", "login": "other-bot", "id": "BOT_2" }
                        ]
                    }
                }
            }
        }"#;

        let agents = parse_suggested_actors_response(body).expect("response should parse");

        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "copilot");
        assert_eq!(agents[0].node_id, "BOT_1");
        assert_eq!(agents[1].name, "other-bot");
        assert_eq!(agents[1].node_id, "BOT_2");
    }

    #[test]
    fn parse_suggested_actors_null_repository_returns_empty_vec() {
        // `suggestedActors` not supported or repo not found — `repository` null.
        let body = r#"{ "data": { "repository": null } }"#;

        let agents =
            parse_suggested_actors_response(body).expect("null repository should be Ok(vec![])");

        assert!(agents.is_empty(), "null repository → empty agent set");
    }

    #[test]
    fn parse_suggested_actors_graphql_error_returns_err() {
        let body = r#"{
            "data": null,
            "errors": [{ "message": "Field 'suggestedActors' doesn't exist" }]
        }"#;

        let error = parse_suggested_actors_response(body)
            .expect_err("a GraphQL error should propagate as Err");

        assert_eq!(error.kind(), AppErrorKind::Internal);
        assert!(
            format!("{error:?}").contains("suggestedActors"),
            "the error context should include the GraphQL message: {error:?}"
        );
    }
}
