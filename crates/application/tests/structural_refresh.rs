//! The **structural** refresh: the issue-index **Sweep** that keeps issue structure
//! within the Freshness contract's 60 seconds, and the **Hydrate** of only what it
//! reports as new or moved (ADR 0008).
//!
//! The cases assert the board a given set of GitHub facts produces, plus the two
//! things the contract promises about cost: which numbers were hydrated, and
//! whether the cache was rewritten. All offline.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use application::{
    BoardCachePort, BoardCacheUsage, BoardOpen, BoardRefresh, BoardSnapshot, CachedBoardService,
    GitHubPort, LoadedBoard,
};
use async_trait::async_trait;
use domain::{
    AppAction, AppError, AppResult, BlockedReason, IssueStamp, LinkedPrRef, PrStatus, Project,
    RawIssue, RawLinkedPr, RepoRef, SliceState,
};

#[derive(Default)]
struct CountingCache {
    snapshots: Mutex<HashMap<String, BoardSnapshot>>,
    writes: AtomicUsize,
}

impl CountingCache {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BoardCachePort for CountingCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        Ok(self
            .snapshots
            .lock()
            .expect("lock poisoned")
            .get(&repo.to_string())
            .cloned())
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .insert(repo.to_string(), snapshot.clone());
        Ok(())
    }

    async fn cache_usage(&self) -> AppResult<BoardCacheUsage> {
        Ok(BoardCacheUsage::default())
    }

    async fn clear_board(&self, _repo: &RepoRef) -> AppAction {
        Ok(())
    }

    async fn clear_all(&self) -> AppAction {
        Ok(())
    }
}

/// A GitHub holding issues the tests edit between refreshes. The index lists the
/// open ones; a **Hydrate** reads whichever exist, closed or not, and records what
/// it was asked for.
#[derive(Default)]
struct LiveGitHub {
    issues: Mutex<BTreeMap<u64, RawIssue>>,
    /// Numbers the index still lists after the issue itself is gone: the window
    /// between GitHub's sweep and hydrate answers.
    listed_but_gone: Mutex<Vec<IssueStamp>>,
    open_prs: Mutex<Vec<RawLinkedPr>>,
    hydrated: Mutex<Vec<Vec<u64>>>,
    sweep_fails: AtomicBool,
    hydrate_fails: AtomicBool,
}

impl LiveGitHub {
    fn with(issues: Vec<RawIssue>) -> Arc<Self> {
        let github = Self::default();
        *github.issues.lock().expect("lock poisoned") =
            issues.into_iter().map(|i| (i.number, i)).collect();
        Arc::new(github)
    }

    /// `issue` is created or edited on GitHub.
    fn put(&self, issue: RawIssue) {
        self.issues
            .lock()
            .expect("lock poisoned")
            .insert(issue.number, issue);
    }

    /// `number` is deleted or transferred out of the repo.
    fn remove(&self, number: u64) {
        self.issues.lock().expect("lock poisoned").remove(&number);
    }

    /// The hydrate calls so far, one entry per call, and forget them.
    fn take_hydrated(&self) -> Vec<Vec<u64>> {
        std::mem::take(&mut *self.hydrated.lock().expect("lock poisoned"))
    }
}

#[async_trait]
impl GitHubPort for LiveGitHub {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .issues
            .lock()
            .expect("lock poisoned")
            .values()
            .filter(|issue| !issue.closed)
            .cloned()
            .collect())
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(self.open_prs.lock().expect("lock poisoned").clone())
    }

    async fn sweep_open_issues(&self, _repo: &RepoRef) -> AppResult<Vec<IssueStamp>> {
        if self.sweep_fails.load(Ordering::SeqCst) {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        let mut index: Vec<IssueStamp> = self
            .issues
            .lock()
            .expect("lock poisoned")
            .values()
            .filter(|issue| !issue.closed)
            .map(|issue| IssueStamp {
                number: issue.number,
                updated_at: issue.updated_at.clone(),
            })
            .collect();
        index.extend(self.listed_but_gone.lock().expect("lock poisoned").clone());
        Ok(index)
    }

    async fn hydrate_issues(&self, _repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        self.hydrated
            .lock()
            .expect("lock poisoned")
            .push(numbers.to_vec());
        if self.hydrate_fails.load(Ordering::SeqCst) {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        let issues = self.issues.lock().expect("lock poisoned");
        Ok(numbers
            .iter()
            .filter_map(|number| issues.get(number).cloned())
            .collect())
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

type Service = CachedBoardService<Arc<LiveGitHub>, Arc<CountingCache>>;

fn repo() -> RepoRef {
    RepoRef::new("funkode-io", "zfirot")
}

fn issue(number: u64) -> RawIssue {
    RawIssue {
        number,
        title: format!("Issue {number}"),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        is_native_child_of_prd: false,
        updated_at: "2026-11-09T10:00:00Z".to_string(),
    }
}

/// `issue` as an edit on GitHub leaves it: `edit` applied, and its stamp moved.
fn edited(issue: RawIssue, edit: impl FnOnce(&mut RawIssue)) -> RawIssue {
    let mut issue = issue;
    edit(&mut issue);
    issue.updated_at = "2026-11-09T10:05:00Z".to_string();
    issue
}

fn prd(number: u64) -> RawIssue {
    RawIssue {
        labels: vec!["prd".to_string()],
        ..issue(number)
    }
}

async fn opened(
    issues: Vec<RawIssue>,
) -> (Arc<LiveGitHub>, Arc<CountingCache>, Service, LoadedBoard) {
    let github = LiveGitHub::with(issues);
    let cache = Arc::new(CountingCache::default());
    let service = CachedBoardService::new(github.clone(), cache.clone());
    let loaded = match service.open(&repo()).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded,
        BoardOpen::Cached(_) => panic!("the first open of a project is cold"),
    };
    (github, cache, service, loaded)
}

async fn refreshed(service: &Service, snapshot: &BoardSnapshot) -> LoadedBoard {
    match service
        .structural_refresh_cached(&repo(), snapshot)
        .await
        .expect("structural refresh should succeed")
    {
        BoardRefresh::Changed(loaded) => loaded,
        BoardRefresh::Unchanged(_) => panic!("this refresh should have changed the board"),
    }
}

fn slice(loaded: &LoadedBoard, number: u64) -> &domain::Slice {
    loaded
        .board
        .slices
        .iter()
        .find(|slice| slice.number == number)
        .unwrap_or_else(|| panic!("#{number} should be a Slice on the board"))
}

fn on_board(loaded: &LoadedBoard, number: u64) -> bool {
    loaded.board.slices.iter().any(|s| s.number == number)
}

#[tokio::test]
async fn a_slice_created_in_grooming_appears_on_its_prds_lane() {
    let (github, _, service, loaded) = opened(vec![prd(1)]).await;

    github.put(RawIssue {
        native_parent: Some(1),
        ..issue(2)
    });

    let after = refreshed(&service, &loaded.snapshot).await;
    assert_eq!(
        slice(&after, 2).prd.as_ref().map(|prd| prd.number),
        Some(1),
        "the new Slice lands under its Parent",
    );
}

#[tokio::test]
async fn a_new_prd_appears_as_a_new_lane() {
    let (github, _, service, loaded) = opened(vec![issue(1)]).await;

    github.put(prd(5));

    let after = refreshed(&service, &loaded.snapshot).await;
    assert!(after.board.prds.iter().any(|prd| prd.number == 5));
}

#[tokio::test]
async fn a_retitle_relabel_or_assignment_is_reflected() {
    struct Case {
        name: &'static str,
        edit: fn(&mut RawIssue),
        check: fn(&domain::Slice) -> bool,
    }
    let cases = [
        Case {
            name: "retitle",
            edit: |issue| issue.title = "Renamed".to_string(),
            check: |slice| slice.title == "Renamed",
        },
        Case {
            name: "relabel",
            edit: |issue| issue.labels.push("needs-triage".to_string()),
            check: |slice| slice.primary_blocked_reason() == Some(BlockedReason::NeedsTriage),
        },
        Case {
            name: "assignment",
            edit: |issue| issue.assignee = Some("colleague".to_string()),
            check: |slice| slice.assignee.as_deref() == Some("colleague"),
        },
    ];

    for case in cases {
        let (github, _, service, loaded) = opened(vec![issue(7)]).await;

        github.put(edited(issue(7), case.edit));

        let after = refreshed(&service, &loaded.snapshot).await;
        assert!((case.check)(slice(&after, 7)), "{}", case.name);
    }
}

#[tokio::test]
async fn a_blocked_reason_added_or_removed_re_derives_the_slice() {
    let (github, _, service, loaded) = opened(vec![issue(8)]).await;
    assert_eq!(slice(&loaded, 8).state, SliceState::Ready);

    github.put(edited(issue(8), |issue| {
        issue.labels.push("deferred".to_string())
    }));
    let deferred = refreshed(&service, &loaded.snapshot).await;
    assert_eq!(slice(&deferred, 8).state, SliceState::Blocked);
    assert_eq!(
        slice(&deferred, 8).primary_blocked_reason(),
        Some(BlockedReason::Deferred),
    );

    let mut unparked = issue(8);
    unparked.updated_at = "2026-11-09T10:10:00Z".to_string();
    github.put(unparked);
    let ready = refreshed(&service, &deferred.snapshot).await;
    assert_eq!(slice(&ready, 8).state, SliceState::Ready);
}

#[tokio::test]
async fn a_prose_blocked_by_entry_added_re_derives_the_slice() {
    let (github, _, service, loaded) = opened(vec![issue(9), issue(10)]).await;

    github.put(edited(issue(10), |issue| {
        issue.body = Some("## Blocked by\n\n- #9\n".to_string())
    }));

    let after = refreshed(&service, &loaded.snapshot).await;
    assert_eq!(slice(&after, 10).state, SliceState::Blocked);
}

#[tokio::test]
async fn a_manually_closed_slice_leaves_the_board_and_readies_its_dependents() {
    let (github, _, service, loaded) = opened(vec![
        issue(11),
        RawIssue {
            native_blockers: vec![11],
            ..issue(12)
        },
    ])
    .await;
    assert_eq!(slice(&loaded, 12).state, SliceState::Blocked);

    github.put(edited(issue(11), |issue| issue.closed = true));

    let after = refreshed(&service, &loaded.snapshot).await;
    assert!(!on_board(&after, 11));
    assert_eq!(slice(&after, 12).state, SliceState::Ready);
    assert_eq!(
        github.take_hydrated(),
        Vec::<Vec<u64>>::new(),
        "absence from the sweep is the whole answer: there is nothing to hydrate",
    );
}

#[tokio::test]
async fn an_issue_absent_from_the_sweep_is_dropped_without_asking_github_about_it() {
    let (github, cache, service, loaded) = opened(vec![issue(13), issue(14)]).await;

    github.remove(14);

    let after = refreshed(&service, &loaded.snapshot).await;
    assert!(on_board(&after, 13));
    assert!(!on_board(&after, 14), "a deleted issue must not linger");
    assert_eq!(github.take_hydrated(), Vec::<Vec<u64>>::new());
    assert_eq!(
        cache
            .cached_board(&repo())
            .await
            .expect("cache read should succeed")
            .expect("the cache holds a snapshot"),
        after.snapshot,
        "the drop is cached, so reopening does not paint a ghost",
    );
}

#[tokio::test]
async fn only_new_or_moved_issues_are_hydrated() {
    let (github, _, service, loaded) = opened(vec![issue(15), issue(16), issue(17)]).await;

    github.put(edited(issue(16), |issue| issue.title = "Moved".to_string()));
    github.put(issue(18));

    refreshed(&service, &loaded.snapshot).await;

    assert_eq!(
        github.take_hydrated(),
        vec![vec![16, 18]],
        "#15 and #17 kept their stamps, so GitHub is not asked about them",
    );
}

#[tokio::test]
async fn an_unchanged_repo_hydrates_nothing_repaints_nothing_and_writes_nothing() {
    let (github, cache, service, loaded) = opened(vec![issue(19), prd(20)]).await;
    let writes_after_seed = cache.writes();

    let refresh = service
        .structural_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect("structural refresh should succeed");

    assert!(
        matches!(refresh, BoardRefresh::Unchanged(_)),
        "an idle repo must not repaint",
    );
    assert_eq!(github.take_hydrated(), Vec::<Vec<u64>>::new());
    assert_eq!(cache.writes(), writes_after_seed);
}

/// A comment moves `updatedAt` without changing anything the board shows. It must
/// not repaint, yet the new stamp has to stick, or every later sweep would hydrate
/// the same issue again.
#[tokio::test]
async fn a_moved_stamp_with_the_same_facts_repaints_nothing_and_is_not_hydrated_twice() {
    let (github, _, service, loaded) = opened(vec![issue(21)]).await;

    github.put(edited(issue(21), |_| {}));

    let first = service
        .structural_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect("structural refresh should succeed");
    let BoardRefresh::Unchanged(adopted) = first else {
        panic!("a stamp alone changes nothing on the board");
    };
    assert_eq!(github.take_hydrated(), vec![vec![21]]);

    service
        .structural_refresh_cached(&repo(), &adopted)
        .await
        .expect("structural refresh should succeed");
    assert_eq!(
        github.take_hydrated(),
        Vec::<Vec<u64>>::new(),
        "the stamp the hydrate read is the one the next sweep compares against",
    );

    // Reopening from disk, where a caller's in-memory stamp is gone, agrees.
    let reopened = match service.open(&repo()).await.expect("open should succeed") {
        BoardOpen::Cached(loaded) => loaded.snapshot,
        BoardOpen::Cold(_) => panic!("the project is cached"),
    };
    service
        .structural_refresh_cached(&repo(), &reopened)
        .await
        .expect("structural refresh should succeed");
    assert_eq!(github.take_hydrated(), Vec::<Vec<u64>>::new());
}

/// The sweep listed it, then it vanished before the hydrate read it: that reads as
/// gone, and the rest of the refresh stands.
#[tokio::test]
async fn a_number_the_hydrate_cannot_resolve_is_gone_not_a_failure() {
    let (github, _, service, loaded) = opened(vec![issue(22), issue(23)]).await;

    github.put(edited(issue(22), |issue| issue.title = "Moved".to_string()));
    github.remove(23);
    github
        .listed_but_gone
        .lock()
        .expect("lock poisoned")
        .push(IssueStamp {
            number: 23,
            updated_at: "2026-11-09T10:05:00Z".to_string(),
        });

    let after = refreshed(&service, &loaded.snapshot).await;
    assert_eq!(slice(&after, 22).title, "Moved");
    assert!(!on_board(&after, 23));
}

#[tokio::test]
async fn a_failed_sweep_leaves_the_last_good_board_and_cache_untouched() {
    let (github, cache, service, loaded) = opened(vec![issue(24)]).await;
    let writes_after_seed = cache.writes();

    github.sweep_fails.store(true, Ordering::SeqCst);

    let error = service
        .structural_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect_err("a failing sweep must surface as an error");
    assert!(
        format!("{error:?}").contains("repo"),
        "the failure carries the repo in its cause chain: {error:?}",
    );
    assert_eq!(cache.writes(), writes_after_seed);
}

/// A failed hydrate must not half-apply the sweep: writing the drop without the
/// edit is harmless, but writing a stamp without its detail would lose the edit
/// for good. Nothing lands, and the next tick finds the same change.
#[tokio::test]
async fn a_failed_hydrate_writes_nothing_and_the_next_tick_finds_the_same_change() {
    let (github, cache, service, loaded) = opened(vec![issue(25), issue(26)]).await;
    let writes_after_seed = cache.writes();

    github.put(edited(issue(25), |issue| issue.title = "Moved".to_string()));
    github.remove(26);
    github.hydrate_fails.store(true, Ordering::SeqCst);

    service
        .structural_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect_err("a failing hydrate must surface as an error");
    assert_eq!(cache.writes(), writes_after_seed);

    github.hydrate_fails.store(false, Ordering::SeqCst);
    let after = refreshed(&service, &loaded.snapshot).await;
    assert_eq!(slice(&after, 25).title, "Moved");
    assert!(!on_board(&after, 26));
}

/// The structural path being down must not hold PR facts hostage: the hot path
/// runs no index sweep at all.
#[tokio::test]
async fn a_failing_structural_refresh_does_not_stop_pr_facts_from_refreshing() {
    let (github, _, service, loaded) = opened(vec![issue(27)]).await;

    github.sweep_fails.store(true, Ordering::SeqCst);
    service
        .structural_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect_err("the structural path is broken");

    *github.open_prs.lock().expect("lock poisoned") = vec![RawLinkedPr {
        pr: LinkedPrRef {
            number: 270,
            author: Some("colleague".to_string()),
            title: "PR 270".to_string(),
            url: "https://github.com/funkode-io/zfirot/pull/270".to_string(),
            pr_status: PrStatus::AwaitingReview,
            conflicts: false,
            ci_failing: false,
            unresolved_comment_count: 0,
        },
        closes: vec![27],
    }];

    let hot = service
        .hot_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect("the hot path must not depend on the issue index");
    let BoardRefresh::Changed(after) = hot else {
        panic!("the new PR changes the board");
    };
    assert_eq!(slice(&after, 27).state, SliceState::Wip);
}
