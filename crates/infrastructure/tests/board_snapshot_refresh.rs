use std::collections::VecDeque;
use std::sync::Mutex;

use application::{classify, BoardRefresh, BoardService, GitHubPort};
use async_trait::async_trait;
use domain::{AgentRef, AppAction, AppResult, Project, RawIssue, RepoRef};
use infrastructure::sample_raw_issues;

struct SequencePort {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    agents: Mutex<VecDeque<Vec<AgentRef>>>,
}

impl SequencePort {
    fn new(issues: Vec<Vec<RawIssue>>, agents: Vec<Vec<AgentRef>>) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from(issues)),
            agents: Mutex::new(VecDeque::from(agents)),
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
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn suggested_agents(&self, _repo: &RepoRef) -> AppResult<Vec<AgentRef>> {
        Ok(self
            .agents
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .expect("agents sequence should have a value"))
    }
}

#[test]
fn classify_is_a_pure_projection_over_raw_issues_and_agents() {
    let raw_issues = sample_raw_issues();
    let agents = vec![AgentRef {
        name: "copilot".to_string(),
        node_id: "BOT_NODE_1".to_string(),
    }];

    let board = classify(&raw_issues, &agents);
    let board_again = classify(&raw_issues, &agents);

    assert_eq!(
        board, board_again,
        "same raw issue set and agents must classify to the same board",
    );
}

#[tokio::test]
async fn refresh_reports_unchanged_when_snapshot_facts_match() {
    let issues = sample_raw_issues();
    let agents = vec![AgentRef {
        name: "copilot".to_string(),
        node_id: "BOT_NODE_1".to_string(),
    }];
    let service = BoardService::new(SequencePort::new(
        vec![issues.clone(), issues],
        vec![agents.clone(), agents],
    ));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let loaded = service.load(&repo).await.expect("load should succeed");
    let refresh = service
        .refresh(&repo, &loaded.snapshot)
        .await
        .expect("refresh should succeed");

    assert_eq!(
        refresh,
        BoardRefresh::Unchanged,
        "equal raw issues + equal agents must not trigger repaint",
    );
}

#[tokio::test]
async fn refresh_reports_changed_when_snapshot_facts_differ() {
    let initial = sample_raw_issues();
    let mut changed = sample_raw_issues();
    changed.retain(|issue| issue.number != 3);
    let service = BoardService::new(SequencePort::new(
        vec![initial, changed],
        vec![vec![], vec![]],
    ));
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
