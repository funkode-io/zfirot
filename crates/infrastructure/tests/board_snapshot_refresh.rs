use std::collections::VecDeque;
use std::sync::Mutex;

use application::{classify, BoardRefresh, BoardService, GitHubPort};
use async_trait::async_trait;
use domain::{
    AppAction, AppResult, IssueStamp, Project, RawIssue, RawLinkedPr, RepoRef, SliceState,
};
use infrastructure::{sample_open_prs, sample_raw_issues};

/// A GitHub that loads `opened` and is found holding `current` by any later
/// **Sweep** or **Hydrate**.
struct SequencePort {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    current: Vec<RawIssue>,
    /// The open-PR **Sweep** every refresh path runs; fixed here, since these
    /// tests are about the issue side of a refresh.
    open_prs: Vec<RawLinkedPr>,
}

impl SequencePort {
    fn new(opened: Vec<RawIssue>, current: Vec<RawIssue>) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from([opened])),
            current,
            open_prs: sample_open_prs(),
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

    async fn sweep_open_issues(&self, _repo: &RepoRef) -> AppResult<Vec<IssueStamp>> {
        Ok(self
            .current
            .iter()
            .filter(|issue| !issue.closed)
            .map(|issue| IssueStamp {
                number: issue.number,
                updated_at: issue.updated_at.clone(),
            })
            .collect())
    }

    async fn hydrate_issues(&self, _repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .current
            .iter()
            .filter(|issue| numbers.contains(&issue.number))
            .cloned()
            .collect())
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(self.open_prs.clone())
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
fn classify_is_a_pure_projection_over_issues_and_swept_prs() {
    let raw_issues = sample_raw_issues();

    let open_prs = sample_open_prs();

    let board = classify(&raw_issues, &open_prs);
    let board_again = classify(&raw_issues, &open_prs);

    assert_eq!(
        board, board_again,
        "same raw issue set must classify to the same board",
    );
}

#[tokio::test]
async fn refresh_reports_unchanged_when_snapshot_facts_match() {
    let issues = sample_raw_issues();
    let service = BoardService::new(SequencePort::new(issues.clone(), issues));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    match refresh {
        BoardRefresh::Unchanged(snapshot) => {
            assert!(
                snapshot.fetched_at > loaded.snapshot.fetched_at,
                "unchanged refresh still reports when it read GitHub",
            );
        }
        BoardRefresh::Changed(_) => panic!("equal raw issues must not trigger repaint"),
    }
}

#[tokio::test]
async fn refresh_reports_changed_when_snapshot_facts_differ() {
    let initial = sample_raw_issues();
    let current: Vec<RawIssue> = initial
        .iter()
        .filter(|issue| issue.number != 3)
        .cloned()
        .collect();
    let service = BoardService::new(SequencePort::new(initial, current));
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
        BoardRefresh::Unchanged(_) => {
            panic!("refresh should report changed when the issue set differs")
        }
    }
}

#[tokio::test]
async fn refresh_rederives_blocked_state_when_blocker_leaves_the_index() {
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
        is_native_child_of_prd: false,
        updated_at: String::new(),
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
        is_native_child_of_prd: false,
        updated_at: String::new(),
    };

    let service = BoardService::new(SequencePort::new(
        vec![blocker, blocked.clone()],
        vec![blocked.clone()],
    ));

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    let updated = match refresh {
        BoardRefresh::Changed(updated) => updated,
        BoardRefresh::Unchanged(_) => panic!("closed blocker should change board state"),
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
        "when its blocker closes, the slice should leave blocked state",
    );
}

#[tokio::test]
async fn a_pr_left_open_when_its_issue_closes_moves_from_the_card_to_unlinked() {
    // Two PRs closed #1: one merged (gone from the sweep) and closed the issue,
    // and the other is still open with a closing reference to a closed issue.
    let open_slice = sample_raw_issues()
        .into_iter()
        .find(|issue| !issue.closed && issue.labels.iter().any(|label| label == "slice"))
        .expect("the sample board should hold an open Slice");
    let number = open_slice.number;
    let mut still_open = sample_open_prs().remove(0);
    still_open.closes = vec![number];
    let still_open_number = still_open.pr.number;
    let service = BoardService::new(SequencePort {
        issues: Mutex::new(VecDeque::from([vec![open_slice]])),
        current: vec![],
        open_prs: vec![still_open],
    });
    let repo = RepoRef::new("funkode-io", "zfirot");

    let loaded = service.load(&repo).await.expect("load should succeed");
    assert_eq!(
        loaded.board.slices[0].linked_prs[0].number,
        still_open_number
    );

    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    let BoardRefresh::Changed(refreshed) = refresh else {
        panic!("closing the issue changes the board");
    };
    assert!(refreshed.board.slices.is_empty());
    let unlinked: Vec<u64> = refreshed
        .board
        .unlinked_prs
        .iter()
        .map(|pr| pr.number)
        .collect();
    assert_eq!(unlinked, vec![still_open_number]);
}
