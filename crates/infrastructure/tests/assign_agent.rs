//! Integration test: the assign-agent (delegate) use-case runs against fake
//! ports that record the delegation (success) and reject it (failure), so both
//! the happy path and the "surface a clear error, leave the board unchanged"
//! path are exercised without GitHub access.

use std::sync::{Arc, Mutex};

use application::{BoardService, GitHubPort};
use async_trait::async_trait;
use domain::{AgentRef, AppAction, AppError, AppErrorKind, AppResult, Project, RawIssue, RepoRef};

/// A fake that records which issue and Agent it was asked to delegate, so the
/// test can assert the use-case forwarded the chosen Agent to the port.
#[derive(Default)]
struct RecordingPort {
    delegated: Mutex<Vec<(u64, AgentRef)>>,
}

#[async_trait]
impl GitHubPort for RecordingPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn assign_agent(
        &self,
        _repo: &RepoRef,
        issue_number: u64,
        agent: &AgentRef,
    ) -> AppAction {
        self.delegated
            .lock()
            .unwrap()
            .push((issue_number, agent.clone()));
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn suggested_agents(&self, _repo: &RepoRef) -> AppResult<Vec<AgentRef>> {
        Ok(vec![])
    }
}

/// A fake that always rejects the delegation, standing in for a token without
/// permission or an Agent that vanished from the assignable set.
struct FailingPort;

#[async_trait]
impl GitHubPort for FailingPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn assign_agent(
        &self,
        _repo: &RepoRef,
        _issue_number: u64,
        _agent: &AgentRef,
    ) -> AppAction {
        Err(AppError::forbidden(
            "The token lacks permission to assign this issue",
        ))
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn suggested_agents(&self, _repo: &RepoRef) -> AppResult<Vec<AgentRef>> {
        Ok(vec![])
    }
}

fn copilot() -> AgentRef {
    AgentRef {
        name: "copilot-swe-agent".to_string(),
        node_id: "BOT_node_id".to_string(),
    }
}

#[tokio::test]
async fn assign_agent_forwards_the_chosen_agent_to_the_port() {
    let port = Arc::new(RecordingPort::default());
    let service = BoardService::new(port.clone());
    let repo = RepoRef::new("funkode-io", "zfirot");
    let agent = copilot();

    service
        .assign_agent(&repo, 42, &agent)
        .await
        .expect("the recording port should accept the delegation");

    let delegated = port.delegated.lock().unwrap().clone();
    assert_eq!(
        delegated,
        vec![(42, agent)],
        "the use-case should delegate issue #42 to the chosen Agent"
    );
}

#[tokio::test]
async fn assign_agent_surfaces_a_clear_error_with_context() {
    let service = BoardService::new(FailingPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let error = service
        .assign_agent(&repo, 7, &copilot())
        .await
        .expect_err("a rejected delegation should surface an error");

    // The error reaches the caller as a clear, client-safe message (so the board
    // can surface it) with the board left unchanged — the use-case performs no
    // board mutation on the failure path.
    assert_eq!(error.kind(), AppErrorKind::Forbidden);
    assert_eq!(
        error.to_string(),
        "The token lacks permission to assign this issue",
        "the failure should surface a clear, client-safe message"
    );

    // The use-case adds the repo and issue as diagnostic context so logs point
    // at the right Slice (context shows in the Debug rendering, not the Display).
    let diagnostic = format!("{error:?}");
    assert!(
        diagnostic.contains("repo=funkode-io/zfirot"),
        "error should carry the repo context: {diagnostic}"
    );
    assert!(
        diagnostic.contains("issue=7"),
        "error should carry the issue context: {diagnostic}"
    );
}
