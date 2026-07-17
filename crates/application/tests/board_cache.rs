use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use application::{BoardCachePort, BoardOpen, BoardRefresh, CachedBoardService, GitHubPort};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{AgentRef, AppAction, AppResult, Project, RawIssue, RepoRef};

#[derive(Default)]
struct CountingBoardCache {
    snapshots: Mutex<HashMap<String, application::BoardSnapshot>>,
    writes: AtomicUsize,
}

impl CountingBoardCache {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BoardCachePort for CountingBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<application::BoardSnapshot>> {
        Ok(self
            .snapshots
            .lock()
            .expect("lock poisoned")
            .get(&repo.to_string())
            .cloned())
    }

    async fn cache_board(
        &self,
        repo: &RepoRef,
        snapshot: &application::BoardSnapshot,
    ) -> AppAction {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .insert(repo.to_string(), snapshot.clone());
        Ok(())
    }
}

struct SequencePort {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    deltas: Mutex<VecDeque<Vec<RawIssue>>>,
    agents: Mutex<VecDeque<Vec<AgentRef>>>,
}

impl SequencePort {
    fn new(
        issues: Vec<Vec<RawIssue>>,
        deltas: Vec<Vec<RawIssue>>,
        agents: Vec<Vec<AgentRef>>,
    ) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from(issues)),
            deltas: Mutex::new(VecDeque::from(deltas)),
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

fn open_issue(number: u64, title: &str) -> RawIssue {
    RawIssue {
        number,
        title: title.to_string(),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        linked_prs: vec![],
        is_native_child_of_prd: false,
    }
}

#[tokio::test]
async fn cold_cache_falls_back_to_load_and_seeds_cache() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(10, "Cold")]],
            vec![vec![]],
            vec![vec![]],
        ),
        cache.clone(),
    );

    let opened = service.open(&repo).await.expect("open should succeed");

    match opened {
        BoardOpen::Cold(loaded) => {
            assert_eq!(
                loaded.board.slices.len(),
                1,
                "cold open should paint loaded board"
            );
        }
        BoardOpen::Cached(_) => panic!("cold cache must load from GitHub"),
    }

    assert_eq!(cache.writes(), 1, "cold open seeds cache once");
    assert!(
        cache
            .cached_board(&repo)
            .await
            .expect("cache read should succeed")
            .is_some(),
        "cold open should persist a snapshot"
    );
}

#[tokio::test]
async fn seeded_open_uses_cache_then_refreshes_and_rewrites_cache() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(20, "Seed")]],
            vec![vec![RawIssue {
                closed: true,
                ..open_issue(20, "Seed")
            }]],
            vec![vec![], vec![]],
        ),
        cache.clone(),
    );

    let seeded = service.open(&repo).await.expect("cold open should seed");
    let snapshot = match seeded {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };

    let cached_open = service
        .open(&repo)
        .await
        .expect("cached open should succeed");
    let cached_snapshot = match cached_open {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices.len(),
                1,
                "cached open paints instantly from cache"
            );
            loaded.snapshot
        }
        BoardOpen::Cold(_) => panic!("seeded project must open from cache"),
    };

    assert_eq!(
        cache.writes(),
        1,
        "cached open should not rewrite cache before refresh"
    );

    let refresh = service
        .refresh_cached(&repo, &cached_snapshot)
        .await
        .expect("refresh should succeed");

    match refresh {
        BoardRefresh::Changed(loaded) => {
            assert!(
                loaded.board.slices.is_empty(),
                "delta should be applied on top of cache"
            );
        }
        BoardRefresh::Unchanged => panic!("closing an issue in delta should change the board"),
    }

    assert!(cache.writes() >= 2, "successful refresh rewrites cache");

    let _ = snapshot;
}

#[tokio::test]
async fn cache_is_scoped_per_repo_for_switch_and_reopen() {
    let repo_a = RepoRef::new("funkode-io", "a");
    let repo_b = RepoRef::new("funkode-io", "b");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(1, "A")], vec![open_issue(2, "B")]],
            vec![vec![]],
            vec![vec![], vec![]],
        ),
        cache.clone(),
    );

    let _ = service
        .open(&repo_a)
        .await
        .expect("first open seeds repo a");
    let _ = service
        .open(&repo_b)
        .await
        .expect("first open seeds repo b");

    let reopened_a = service
        .open(&repo_a)
        .await
        .expect("reopen should use cache");
    match reopened_a {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices[0].number, 1,
                "reopen should paint repo A cache"
            );
        }
        BoardOpen::Cold(_) => panic!("seeded repo a should reopen from cache"),
    }

    let switched_b = service
        .open(&repo_b)
        .await
        .expect("switch should use cache");
    match switched_b {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices[0].number, 2,
                "switch should paint repo B cache"
            );
        }
        BoardOpen::Cold(_) => panic!("seeded repo b should open from cache"),
    }
}
