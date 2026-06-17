//! Use-case tests: `classify_board` discovers Assignable Agents via the port
//! and carries them on the classified board, with best-effort semantics.
//!
//! All tests run against fake ports returning canned data — no network access.

use application::{BoardService, ClassifiedBoard, GitHubPort};
use async_trait::async_trait;
use domain::{AgentRef, AppAction, AppError, AppResult, Project, RawIssue, RepoRef};

/// Build a minimal fake port that returns `agents` from `suggested_agents` and
/// empty lists everywhere else.
struct AgentPort {
    agents: AppResult<Vec<AgentRef>>,
}

impl AgentPort {
    fn returning(agents: Vec<AgentRef>) -> Self {
        Self { agents: Ok(agents) }
    }

    fn failing() -> Self {
        Self {
            agents: Err(AppError::unavailable("GitHub is down")),
        }
    }
}

#[async_trait]
impl GitHubPort for AgentPort {
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
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn suggested_agents(&self, _repo: &RepoRef) -> AppResult<Vec<AgentRef>> {
        // `AppError` is not `Clone`, so the failing case returns a fresh
        // deterministic `Unavailable` error rather than re-wrapping the stored
        // one (which would stringify it and flatten its kind/context).
        match &self.agents {
            Ok(agents) => Ok(agents.clone()),
            Err(_) => Err(AppError::unavailable("GitHub is down")),
        }
    }
}

#[tokio::test]
async fn classify_board_carries_zero_agents_when_none_are_discovered() {
    let service = BoardService::new(AgentPort::returning(vec![]));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { agents, .. } = service
        .classify_board(&repo)
        .await
        .expect("board should classify with zero agents");

    assert!(agents.is_empty(), "no bots discovered → empty agent set");
}

#[tokio::test]
async fn classify_board_carries_one_agent_on_the_board() {
    let copilot = AgentRef {
        name: "copilot".to_string(),
        node_id: "BOT_NODE_1".to_string(),
    };
    let service = BoardService::new(AgentPort::returning(vec![copilot.clone()]));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { agents, .. } = service
        .classify_board(&repo)
        .await
        .expect("board should classify with one agent");

    assert_eq!(
        agents,
        vec![copilot],
        "single discovered agent should be on the board"
    );
}

#[tokio::test]
async fn classify_board_carries_many_agents_on_the_board() {
    let bot_a = AgentRef {
        name: "copilot".to_string(),
        node_id: "BOT_1".to_string(),
    };
    let bot_b = AgentRef {
        name: "other-bot".to_string(),
        node_id: "BOT_2".to_string(),
    };
    let service = BoardService::new(AgentPort::returning(vec![bot_a.clone(), bot_b.clone()]));
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { agents, .. } = service
        .classify_board(&repo)
        .await
        .expect("board should classify with many agents");

    assert_eq!(
        agents,
        vec![bot_a, bot_b],
        "all discovered agents should be on the board"
    );
}

#[tokio::test]
async fn classify_board_degrades_to_empty_agents_when_discovery_fails() {
    let service = BoardService::new(AgentPort::failing());
    let repo = RepoRef::new("funkode-io", "zfirot");

    // The board must still classify successfully — the Slices/PRDs/other are
    // unaffected and the agents field is simply empty.
    let board = service
        .classify_board(&repo)
        .await
        .expect("board should classify even when agent discovery fails");

    assert!(
        board.agents.is_empty(),
        "a discovery failure should degrade to an empty agent set, not sink the board"
    );
}
