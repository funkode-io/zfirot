use std::collections::VecDeque;
use std::sync::Mutex;

use application::{classify, BoardRefresh, BoardService, GitHubPort};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{AppAction, AppResult, Project, RawIssue, RepoRef, SliceState};
use infrastructure::sample_raw_issues;

struct SequencePort {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    deltas: Mutex<VecDeque<Vec<RawIssue>>>,
}

impl SequencePort {
    fn new(issues: Vec<Vec<RawIssue>>, deltas: Vec<Vec<RawIssue>>) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from(issues)),
            deltas: Mutex::new(VecDeque::from(deltas)),
        }
    }
}

#[async_trait]
impl GitHubPort for SequencePort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .issues
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .expect("issues sequence should have a value"))
    }

    async fn load_issues_since(
        &self,
        _repo: &RepoRef,
        _since: DateTime<Utc>,
    ) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .deltas
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .expect("delta sequence should have a value"))
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }
}

#[test]
fn classify_is_a_pure_projection_over_raw_issues() {
    let raw_issues = sample_raw_issues();

    let board = classify(&raw_issues);
    let board_again = classify(&raw_issues);

    assert_eq!(
        board, board_again,
        "same raw issue set must classify to the same board",
    );
}

#[tokio::test]
async fn refresh_reports_unchanged_when_snapshot_facts_match() {
    let issues = sample_raw_issues();
    let service = BoardService::new(SequencePort::new(vec![issues], vec![vec![]]));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    assert_eq!(
        refresh,
        BoardRefresh::Unchanged,
        "equal raw issues must not trigger repaint",
    );
}

#[tokio::test]
async fn refresh_reports_changed_when_snapshot_facts_differ() {
    let initial = sample_raw_issues();
    let closed_three = RawIssue {
        number: 3,
        title: "Derive SliceState as a pure domain function".to_string(),
        url: "https://github.com/funkode-io/zfirot/issues/3".to_string(),
        body: None,
        labels: vec!["ready-for-agent".to_string()],
        closed: true,
        native_parent: Some(1),
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        linked_prs: vec![],
        is_native_child_of_prd: true,
    };
    let service = BoardService::new(SequencePort::new(vec![initial], vec![vec![closed_three]]));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    match refresh {
        BoardRefresh::Changed(view) => {
            assert!(
                view.board.slices.iter().all(|slice| slice.number != 3),
                "changed view should reflect newly fetched issue set",
            );
        }
        BoardRefresh::Unchanged => {
            panic!("refresh should report changed when the issue set differs")
        }
    }
}

#[tokio::test]
async fn refresh_rederives_blocked_state_when_blocker_closes_in_delta() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let blocker = RawIssue {
        number: 42,
        title: "Blocker".to_string(),
        url: "https://github.com/funkode-io/zfirot/issues/42".to_string(),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        linked_prs: vec![],
        is_native_child_of_prd: false,
    };
    let blocked = RawIssue {
        number: 43,
        title: "Blocked".to_string(),
        url: "https://github.com/funkode-io/zfirot/issues/43".to_string(),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![42],
        assignee: None,
        assignee_avatar_url: None,
        linked_prs: vec![],
        is_native_child_of_prd: false,
    };
    let closed_blocker = RawIssue {
        closed: true,
        ..blocker.clone()
    };

    let service = BoardService::new(SequencePort::new(
        vec![vec![blocker, blocked.clone()]],
        vec![vec![closed_blocker]],
    ));

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    let updated = match refresh {
        BoardRefresh::Changed(updated) => updated,
        BoardRefresh::Unchanged => panic!("closed blocker should change board state"),
    };

    let refreshed_blocked = updated
        .board
        .slices
        .iter()
        .find(|slice| slice.number == blocked.number)
        .expect("blocked issue should remain after blocker closes");
    assert_ne!(
        refreshed_blocked.state,
        SliceState::Blocked,
        "when blocker closes in delta, slice should leave blocked state",
    );
}

#[tokio::test]
async fn refresh_is_idempotent_when_since_overlap_refetches_same_issue() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let initial = sample_raw_issues();
    let refetched = initial
        .iter()
        .find(|issue| issue.number == 3)
        .expect("fixture should include issue 3")
        .clone();
    let service = BoardService::new(SequencePort::new(vec![initial], vec![vec![refetched]]));

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    assert_eq!(
        refresh,
        BoardRefresh::Unchanged,
        "overlap deltas that refetch unchanged issues should be idempotent",
    );
}
