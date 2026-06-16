//! Integration test: the confirm-classification use-case runs against fake
//! ports that record the labelling (success) and reject it (failure), so both
//! the happy path and the "surface a clear error, leave the issue unchanged"
//! path are exercised without GitHub access. A classification with nothing to
//! confirm is rejected before the port is ever called.

use std::sync::{Arc, Mutex};

use application::{BoardService, GitHubPort};
use async_trait::async_trait;
use domain::{
    AppAction, AppError, AppErrorKind, AppResult, IssueClassification, Project, RawIssue, RepoRef,
    Slice,
};

/// A fake that records each `(issue_number, label)` it was asked to label, so
/// the test can assert the use-case forwarded the right label for a suggestion.
#[derive(Default)]
struct RecordingPort {
    labelled: Mutex<Vec<(u64, String)>>,
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

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, issue_number: u64, label: &str) -> AppAction {
        self.labelled
            .lock()
            .unwrap()
            .push((issue_number, label.to_string()));
        Ok(())
    }
}

/// A fake that always rejects the labelling, standing in for a token without
/// write permission or a vanished issue.
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
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Err(AppError::forbidden(
            "The token lacks permission to label this issue",
        ))
    }
}

#[tokio::test]
async fn confirming_a_suggested_prd_adds_the_prd_label() {
    let port = Arc::new(RecordingPort::default());
    let service = BoardService::new(port.clone());
    let repo = RepoRef::new("funkode-io", "zfirot");

    service
        .confirm_classification(&repo, 42, &IssueClassification::SuggestedPrd)
        .await
        .expect("the recording port should accept the labelling");

    let labelled = port.labelled.lock().unwrap().clone();
    assert_eq!(
        labelled,
        vec![(42, "prd".to_string())],
        "a suggested PRD should add the `prd` label to issue #42"
    );
}

#[tokio::test]
async fn confirming_a_suggested_slice_adds_the_slice_label() {
    let port = Arc::new(RecordingPort::default());
    let service = BoardService::new(port.clone());
    let repo = RepoRef::new("funkode-io", "zfirot");

    service
        .confirm_classification(&repo, 7, &IssueClassification::SuggestedSlice)
        .await
        .expect("the recording port should accept the labelling");

    let labelled = port.labelled.lock().unwrap().clone();
    assert_eq!(
        labelled,
        vec![(7, "slice".to_string())],
        "a suggested Slice should add the `slice` label to issue #7"
    );
}

#[tokio::test]
async fn confirming_an_unclassified_issue_is_rejected_without_calling_the_port() {
    let port = Arc::new(RecordingPort::default());
    let service = BoardService::new(port.clone());
    let repo = RepoRef::new("funkode-io", "zfirot");

    let error = service
        .confirm_classification(&repo, 1, &IssueClassification::Unclassified)
        .await
        .expect_err("an issue with no suggestion has nothing to confirm");

    // Validated at the use-case boundary, before any GitHub call.
    assert_eq!(error.kind(), AppErrorKind::InvalidInput);
    assert!(
        port.labelled.lock().unwrap().is_empty(),
        "the port must not be called when there is nothing to confirm"
    );
}

#[tokio::test]
async fn confirm_surfaces_a_clear_error_with_context() {
    let service = BoardService::new(FailingPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let error = service
        .confirm_classification(&repo, 7, &IssueClassification::SuggestedSlice)
        .await
        .expect_err("a rejected labelling should surface an error");

    // The error reaches the caller as a clear, client-safe message (so the board
    // can surface it) and the issue is left unchanged — no board mutation.
    assert_eq!(error.kind(), AppErrorKind::Forbidden);
    assert_eq!(
        error.to_string(),
        "The token lacks permission to label this issue",
        "the failure should surface a clear, client-safe message"
    );

    // The use-case adds the repo, issue, and label as diagnostic context so logs
    // point at the right issue (context shows in Debug, not Display).
    let diagnostic = format!("{error:?}");
    assert!(
        diagnostic.contains("repo=funkode-io/zfirot"),
        "error should carry the repo context: {diagnostic}"
    );
    assert!(
        diagnostic.contains("issue=7"),
        "error should carry the issue context: {diagnostic}"
    );
    assert!(
        diagnostic.contains("label=slice"),
        "error should carry the label context: {diagnostic}"
    );
}
