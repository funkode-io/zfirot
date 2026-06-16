//! Integration test: the assign-self use-case runs against fake ports that
//! record the assignment (success) and reject it (failure), so both the happy
//! path and the "surface a clear error, leave the board unchanged" path are
//! exercised without GitHub access.

use std::sync::{Arc, Mutex};

use application::{BoardService, GitHubPort};
use async_trait::async_trait;
use domain::{AppAction, AppError, AppErrorKind, AppResult, Project, RawIssue, RepoRef, Slice};

/// A fake that records which issue it was asked to assign, so the test can
/// assert the use-case forwarded the right number to the port.
#[derive(Default)]
struct RecordingPort {
    assigned: Mutex<Vec<u64>>,
}

#[async_trait]
impl GitHubPort for RecordingPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(vec![])
    }

    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, issue_number: u64) -> AppAction {
        self.assigned.lock().unwrap().push(issue_number);
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }
}

/// A fake that always rejects the assignment, standing in for a token without
/// permission or a vanished issue.
struct FailingPort;

#[async_trait]
impl GitHubPort for FailingPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(vec![])
    }

    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Err(AppError::forbidden(
            "The token lacks permission to assign this issue",
        ))
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Err(AppError::forbidden(
            "The token lacks permission to label this issue",
        ))
    }
}

#[tokio::test]
async fn assign_self_forwards_the_issue_number_to_the_port() {
    let port = Arc::new(RecordingPort::default());
    let service = BoardService::new(port.clone());
    let repo = RepoRef::new("funkode-io", "zfirot");

    service
        .assign_self(&repo, 42)
        .await
        .expect("the recording port should accept the assignment");

    let assigned = port.assigned.lock().unwrap().clone();
    assert_eq!(assigned, vec![42], "the use-case should assign issue #42");
}

#[tokio::test]
async fn assign_self_surfaces_a_clear_error_with_context() {
    let service = BoardService::new(FailingPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let error = service
        .assign_self(&repo, 7)
        .await
        .expect_err("a rejected assignment should surface an error");

    // The error reaches the caller as a clear, client-safe message (so the board
    // can surface it) and is left unchanged — the use-case performs no board
    // mutation on the failure path.
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
