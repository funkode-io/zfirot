//! Application layer: use-cases and the port traits that infrastructure
//! implements (dependency inversion). Depends only on `domain`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    classify_issue, classify_on_board, has_unreferenced_blocked_by, has_veto, lane_parent_of,
    parse_blockers_from_body, resolve_unblocks, AppAction, AppError, AppResult, BoardViewMode,
    DependencyRef, GitHubToken, IssueClassification, IssueStamp, LinkedPrRef, Prd, PrdRef, Project,
    RawIssue, RawLinkedPr, RawSlice, RepoRef, Slice, ThemePreference, Viewer,
};
use serde::{Deserialize, Serialize};

/// The seam between the application and any GitHub backend (real or fake).
///
/// This is the primary test seam: use-cases run against a fake implementation
/// returning canned data, with no network access.
#[async_trait]
pub trait GitHubPort: Send + Sync {
    /// Load the project's issues as raw, GitHub-shaped data, including closed
    /// ones.
    ///
    /// The application layer classifies these and builds the board from them.
    /// The adapter provides both the native-link fields (`native_parent`,
    /// `native_blockers`, `is_native_child_of_prd`) and the raw `body` for the
    /// prose-fallback parsing. Closed issues are included (`RawIssue.closed`);
    /// `classify_board` is responsible for omitting them. It carries **no Pull
    /// Request data**: that is what [`sweep_open_prs`](Self::sweep_open_prs) is
    /// for.
    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>>;

    /// **Sweep** every currently-open Pull Request of the repo, with the facts a
    /// Linked PR renders and the issue numbers it closes.
    ///
    /// The result is *authoritative* over the open-PR set: a PR that has been
    /// merged or closed is simply absent from it, so nothing has to filter it
    /// out afterwards (ADR 0008). The **hot** refresh runs it every 15 seconds,
    /// and [`classify`] joins its result onto the issues by issue number.
    async fn sweep_open_prs(&self, repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>>;

    /// **Sweep** the repo's issue index: every open issue's number and last-updated
    /// stamp, with no other detail.
    ///
    /// The result is *authoritative* over which issues exist: one absent from it
    /// has been closed, deleted or transferred, and is dropped from the board. The
    /// **structural** refresh runs it every 60 seconds and **Hydrates** only the
    /// numbers that are new or whose stamp moved (ADR 0008).
    ///
    /// The default is a **safe full-load fallback**: it derives the index from
    /// [`load_issues`](Self::load_issues), which is correct but forfeits the sweep's
    /// shallowness. Adapters override this to fetch just the index.
    async fn sweep_open_issues(&self, repo: &RepoRef) -> AppResult<Vec<IssueStamp>> {
        Ok(self
            .load_issues(repo)
            .await?
            .into_iter()
            .filter(|issue| !issue.closed)
            .map(|issue| IssueStamp {
                number: issue.number,
                updated_at: issue.updated_at,
            })
            .collect())
    }

    /// **Hydrate** specific issues by number: their full detail, in as few
    /// requests as the adapter can keep inside GitHub's node limit. `numbers` is
    /// sorted and free of duplicates.
    ///
    /// A number GitHub cannot resolve (a deleted or transferred issue) is simply
    /// absent from the answer rather than a failure: absence means gone. An issue
    /// that still exists comes back with its current state, closed or not.
    ///
    /// The default is a **safe full-load fallback**: it delegates to
    /// [`load_issues`](Self::load_issues) and keeps the requested numbers. That
    /// answer may leave out a closed issue instead of reporting it closed, which
    /// means the same thing here. Adapters override this to fetch just those
    /// issues.
    async fn hydrate_issues(&self, repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        let mut issues = self.load_issues(repo).await?;
        issues.retain(|issue| numbers.binary_search(&issue.number).is_ok());
        Ok(issues)
    }

    /// List the repositories the token can access, for the home screen. Ordering
    /// is the adapter's best effort; [`ProjectsService`] re-sorts by recency.
    async fn list_projects(&self) -> AppResult<Vec<Project>>;

    /// Assign the authenticated user to an issue (the Slice's underlying issue),
    /// so picking up a Ready Slice from the board claims it on GitHub.
    async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction;

    /// Add a label to an issue, so confirming a suggested classification tags it
    /// (`prd` or `slice`) and the next poll reclassifies it onto the board.
    async fn add_label(&self, repo: &RepoRef, issue_number: u64, label: &str) -> AppAction;

    /// The authenticated Viewer (login, name, avatar), for the account menu.
    ///
    /// Defaults to a clear failure so adapters that do not (yet) support it need
    /// no change; [`AccountService::viewer`] simply propagates that error, and
    /// the presentation layer treats any failure as "viewer unknown" — showing
    /// a generic icon rather than blocking on it.
    async fn viewer(&self) -> AppResult<Viewer> {
        Err(AppError::internal("This adapter cannot fetch the viewer")
            .with_operation("GitHubPort::viewer"))
    }
}

/// Shared ports are ports too, so the composition root can hand the same
/// `Arc<dyn GitHubPort>` to a [`BoardService`] (and clone it into contexts).
#[async_trait]
impl<P: GitHubPort + ?Sized> GitHubPort for Arc<P> {
    async fn load_issues(&self, repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        (**self).load_issues(repo).await
    }

    async fn sweep_open_prs(&self, repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        (**self).sweep_open_prs(repo).await
    }

    async fn sweep_open_issues(&self, repo: &RepoRef) -> AppResult<Vec<IssueStamp>> {
        (**self).sweep_open_issues(repo).await
    }

    async fn hydrate_issues(&self, repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        (**self).hydrate_issues(repo, numbers).await
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        (**self).list_projects().await
    }

    async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction {
        (**self).assign_self(repo, issue_number).await
    }

    async fn add_label(&self, repo: &RepoRef, issue_number: u64, label: &str) -> AppAction {
        (**self).add_label(repo, issue_number, label).await
    }

    async fn viewer(&self) -> AppResult<Viewer> {
        (**self).viewer().await
    }
}

/// An issue that is not a confirmed Slice — shown in the "other open issues"
/// section of the board.
///
/// The `classification` field drives the "looks like a PRD/Slice — confirm?"
/// badge for [`IssueClassification::SuggestedPrd`] and
/// [`IssueClassification::SuggestedSlice`] issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherIssue {
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// [`IssueClassification::SuggestedPrd`], [`IssueClassification::SuggestedSlice`],
    /// or [`IssueClassification::Unclassified`].
    pub classification: IssueClassification,
}

/// The result of classifying all open issues in a project.
///
/// - `slices` — tier-1 confirmed Slices and issues promoted by their Parent,
///   ready for the Kanban columns.
/// - `prds`   — tier-1 confirmed PRDs (display is deferred to a later slice).
/// - `other`  — suggested and unclassified issues with no promoting Parent, for
///   the "other open issues" bucket. A Suggested PRD stays here even while it
///   heads a Lane, since this is where it can be confirmed.
/// - `unlinked_prs` — **Unlinked PRs**: open PRs on no Slice card, in sweep order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedBoard {
    pub slices: Vec<Slice>,
    pub prds: Vec<Prd>,
    pub other: Vec<OtherIssue>,
    pub unlinked_prs: Vec<LinkedPrRef>,
}

/// The stored format of a [`BoardSnapshot`].
///
/// Bumped whenever the snapshot's serde shape changes. A cached snapshot from
/// another version fails to deserialize, so the cache adapter discards it and
/// the board cold-loads instead of being mis-read (ADR 0008). Version 2 is the
/// split into two independently-owned collections — issues and Linked PRs.
/// Version 3 adds the issues still awaiting their close. Version 4 stamps each
/// issue with the `updatedAt` an index **Sweep** compares.
const SNAPSHOT_FORMAT_VERSION: u32 = 4;

/// Reject a stored snapshot written by another version of the format, so an
/// older shape is discarded rather than silently half-read.
fn deserialize_format_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version != SNAPSHOT_FORMAT_VERSION {
        return Err(serde::de::Error::custom(format!(
            "cached board snapshot format {version} is not {SNAPSHOT_FORMAT_VERSION}"
        )));
    }
    Ok(version)
}

/// A retained fetch snapshot of a board, used to decide whether a refresh would
/// repaint anything.
///
/// It holds the board's two independently-owned collections — the open issues
/// and the open Linked PRs swept from GitHub — joined only at paint time by the
/// pure [`classify`]. Keeping them apart is what stops a slower issue write from
/// reverting a fresher PR sweep (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardSnapshot {
    /// The stored format version, checked on read (see
    /// [`SNAPSHOT_FORMAT_VERSION`]).
    #[serde(deserialize_with = "deserialize_format_version")]
    version: u32,
    raw_issues: Vec<RawIssue>,
    /// Every open Linked PR of the repo, as the last sweep reported it.
    linked_prs: Vec<RawLinkedPr>,
    /// The issues a **Cascade** read as still open after the PR closing them left
    /// the sweep, and that the next hot refresh reads once more. A merged PR leaves
    /// the sweep at once but GitHub closes its issues a moment later (1-3s,
    /// measured), so a sweep landing in that gap reads too early, and the sweep
    /// after it looks unchanged. Bookkeeping, not a board fact: it never repaints.
    awaiting_close: Vec<u64>,
    /// When the issue side last read GitHub, captured at fetch start. Only ever
    /// shown to the user: what needs re-reading is decided by each issue's own
    /// stamp, not by this.
    pub fetched_at: DateTime<Utc>,
}

impl BoardSnapshot {
    fn new(
        raw_issues: Vec<RawIssue>,
        linked_prs: Vec<RawLinkedPr>,
        fetched_at: DateTime<Utc>,
    ) -> Self {
        Self {
            version: SNAPSHOT_FORMAT_VERSION,
            raw_issues,
            linked_prs,
            awaiting_close: Vec::new(),
            fetched_at,
        }
    }

    /// Whether the two paint the same board. An issue's stamp is deliberately not
    /// a fact: a comment moves it and changes nothing on screen, so it must not
    /// repaint. Closed issues paint nothing either.
    fn same_facts_as(&self, other: &Self) -> bool {
        self.linked_prs == other.linked_prs
            && same_open_issue_facts(&self.raw_issues, &other.raw_issues)
    }

    /// This snapshot with its **issue** collection replaced — the write of the
    /// structural and safety-net paths, which own the issues and nothing else.
    fn with_issues(&self, raw_issues: Vec<RawIssue>, fetched_at: DateTime<Utc>) -> Self {
        Self {
            awaiting_close: self.awaiting_close.clone(),
            ..Self::new(raw_issues, self.linked_prs.clone(), fetched_at)
        }
    }

    /// This snapshot with its **Linked PR** collection replaced — the hot path's
    /// own write. `fetched_at` does not move, since it sweeps no issues.
    fn with_linked_prs(&self, linked_prs: Vec<RawLinkedPr>) -> Self {
        Self {
            awaiting_close: self.awaiting_close.clone(),
            ..Self::new(self.raw_issues.clone(), linked_prs, self.fetched_at)
        }
    }

    /// This snapshot with a **Cascade**'s answer written over the issues it asked
    /// for. `fetched_at` stays put: those few issues say nothing about the rest.
    fn with_hydrated_issues(&self, requested: &[u64], hydrated: &[RawIssue]) -> Self {
        Self {
            awaiting_close: self.awaiting_close.clone(),
            ..Self::new(
                merge_hydrated_issues(&self.raw_issues, requested, hydrated),
                self.linked_prs.clone(),
                self.fetched_at,
            )
        }
    }

    /// The board this snapshot paints: the pure join of its two collections.
    fn loaded(self) -> LoadedBoard {
        LoadedBoard {
            board: classify(&self.raw_issues, &self.linked_prs),
            snapshot: self,
        }
    }
}

/// Cache usage for one project snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedProjectUsage {
    pub repo: RepoRef,
    pub bytes: u64,
}

/// The board cache footprint grouped by project, plus the summed total.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardCacheUsage {
    pub projects: Vec<CachedProjectUsage>,
    pub total_bytes: u64,
}

/// The loaded board view plus its retained snapshot for refresh decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBoard {
    pub board: ClassifiedBoard,
    pub snapshot: BoardSnapshot,
}

/// Outcome of refreshing a retained board snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardRefresh {
    /// The fetched facts differ from the retained snapshot: repaint with `view`.
    Changed(LoadedBoard),
    /// The fetched facts match the retained snapshot: keep the current UI as-is,
    /// but adopt this snapshot, which may carry stamps that moved without changing
    /// anything on screen.
    Unchanged(BoardSnapshot),
}

/// Outcome of a manual **Refresh**, which runs every refresh pass once (see
/// [`CachedBoardService::full_refresh_cached`]).
#[derive(Debug)]
pub struct FullRefresh {
    /// What the passes found together, judged against the board on screen.
    pub refresh: BoardRefresh,
    /// Every pass that failed, in the order they ran. A failed pass wrote nothing,
    /// and the passes after it still ran.
    pub failures: Vec<AppError>,
}

/// Whether a freshly-written snapshot is worth repainting, given the retained
/// one it was derived from. Each refresh path writes exactly one of the two
/// collections and hands the result here, so "did anything change?" is decided
/// in one place for all of them.
fn refresh_outcome(refreshed: BoardSnapshot, retained: &BoardSnapshot) -> BoardRefresh {
    if refreshed.same_facts_as(retained) {
        return BoardRefresh::Unchanged(refreshed);
    }
    BoardRefresh::Changed(refreshed.loaded())
}

/// Whether two sweeps report the same open-PR **set**. The sweep is ordered by
/// `updatedAt`, so any bump on GitHub reshuffles it without changing a single
/// fact on the board: order alone is not news. A real change repaints with the
/// new order, which is how the Unlinked PR list catches up.
fn same_open_prs(left: &[RawLinkedPr], right: &[RawLinkedPr]) -> bool {
    let by_number: HashMap<u64, &RawLinkedPr> = right
        .iter()
        .map(|linked| (linked.pr.number, linked))
        .collect();
    left.len() == right.len()
        && left
            .iter()
            .all(|linked| by_number.get(&linked.pr.number) == Some(&linked))
}

/// Whether two issue collections hold the same open issues in the same order,
/// stamps aside (see [`BoardSnapshot::same_facts_as`]).
fn same_open_issue_facts(left: &[RawIssue], right: &[RawIssue]) -> bool {
    let open = |issues: &'_ [RawIssue]| -> Vec<RawIssue> {
        issues
            .iter()
            .filter(|issue| !issue.closed)
            .map(|issue| RawIssue {
                updated_at: String::new(),
                ..issue.clone()
            })
            .collect()
    };
    open(left) == open(right)
}

/// The numbers an index **Sweep** says must be **Hydrated**: those new to the
/// board or whose stamp moved. Sorted, as [`GitHubPort::hydrate_issues`] expects.
fn stale_issues(retained: &[RawIssue], index: &[IssueStamp]) -> Vec<u64> {
    let known: HashMap<u64, &str> = retained
        .iter()
        .filter(|issue| !issue.closed)
        .map(|issue| (issue.number, issue.updated_at.as_str()))
        .collect();
    let stale: BTreeSet<u64> = index
        .iter()
        .filter(|stamp| known.get(&stamp.number) != Some(&stamp.updated_at.as_str()))
        .map(|stamp| stamp.number)
        .collect();
    stale.into_iter().collect()
}

fn merge_issues(retained: &[RawIssue], fetched: &[RawIssue]) -> Vec<RawIssue> {
    let mut merged: Vec<RawIssue> = retained.to_vec();
    for issue in fetched {
        if let Some(position) = merged
            .iter()
            .position(|current| current.number == issue.number)
        {
            if issue.closed {
                merged.remove(position);
            } else {
                merged[position] = issue.clone();
            }
        } else if !issue.closed {
            merged.push(issue.clone());
        }
    }
    merged
}

/// The issues a **Cascade** must hydrate: those on either end of a closing edge
/// that one sweep has and the other does not. That covers a PR that appeared, a
/// PR that vanished (whose issues only the previous set still names) and a PR
/// retargeted to another issue. A PR whose status alone changed moves no edge and
/// says nothing about its issue, so it hydrates nothing. Sorted, as
/// [`GitHubPort::hydrate_issues`] expects.
fn cascade_issues(previous: &[RawLinkedPr], swept: &[RawLinkedPr]) -> Vec<u64> {
    fn closing_edges(linked_prs: &[RawLinkedPr]) -> HashSet<(u64, u64)> {
        linked_prs
            .iter()
            .flat_map(|linked| linked.closes.iter().map(|issue| (linked.pr.number, *issue)))
            .collect()
    }

    let (previous, swept) = (closing_edges(previous), closing_edges(swept));
    let moved: BTreeSet<u64> = previous
        .symmetric_difference(&swept)
        .map(|(_, issue)| *issue)
        .collect();
    moved.into_iter().collect()
}

/// The issues to read once more on the next hot refresh: those still open on the
/// `merged` board that were closed by a PR gone from the sweep, or whose earlier
/// look was `undone` because the read was discarded. Only a PR that left the sweep
/// can be a merge; one merely retargeted is still open, so no close is on the way.
/// Judged on the merged issues, so a read an issue-side write discarded still counts
/// when that write left the issue open. Sorted, as [`GitHubPort::hydrate_issues`]
/// expects.
fn issues_awaiting_close(
    previous: &[RawLinkedPr],
    swept: &[RawLinkedPr],
    undone: impl IntoIterator<Item = u64>,
    merged: &[RawIssue],
) -> Vec<u64> {
    let still_swept: HashSet<u64> = swept.iter().map(|linked| linked.pr.number).collect();
    let open: HashSet<u64> = merged
        .iter()
        .filter(|issue| !issue.closed)
        .map(|issue| issue.number)
        .collect();
    let awaiting: BTreeSet<u64> = previous
        .iter()
        .filter(|linked| !still_swept.contains(&linked.pr.number))
        .flat_map(|linked| linked.closes.iter().copied())
        .chain(undone)
        .filter(|issue| open.contains(issue))
        .collect();
    awaiting.into_iter().collect()
}

/// `retained` with a hydrate's answer applied: every issue it returned is merged
/// by number, and a requested number it did not return is gone (ADR 0008).
fn merge_hydrated_issues(
    retained: &[RawIssue],
    requested: &[u64],
    hydrated: &[RawIssue],
) -> Vec<RawIssue> {
    let answered: HashSet<u64> = hydrated.iter().map(|issue| issue.number).collect();
    let survivors: Vec<RawIssue> = retained
        .iter()
        .filter(|issue| answered.contains(&issue.number) || !requested.contains(&issue.number))
        .cloned()
        .collect();
    merge_issues(&survivors, hydrated)
}

/// The numbers of the issues `after` holds differently from `before`, including
/// any one of them holds and the other does not: what was written between two
/// reads of the cache. A stamp alone is no such write: it says nothing the other
/// path's answer could undo, and treating it as one would throw that answer away.
fn issues_changed_between(before: &[RawIssue], after: &[RawIssue]) -> HashSet<u64> {
    let by_number = |issues: &[RawIssue]| -> HashMap<u64, RawIssue> {
        issues
            .iter()
            .map(|issue| {
                let unstamped = RawIssue {
                    updated_at: String::new(),
                    ..issue.clone()
                };
                (issue.number, unstamped)
            })
            .collect()
    };
    let (before, after) = (by_number(before), by_number(after));
    before
        .keys()
        .chain(after.keys())
        .filter(|number| before.get(number) != after.get(number))
        .copied()
        .collect()
}

/// An issue-side result with the issues another path wrote while it was in flight
/// (`written`) laid back over it: a **Cascade**, or the other issue-side path. That
/// write wins: this fetch may have read the issue before the PR merged, or before a
/// safety net dropped or corrected it, and would otherwise put a closed Slice or an
/// older issue back on the board. Issues keep their positions, so the overlay alone
/// never reorders cards.
fn with_issues_written_meanwhile(
    fetched: &[RawIssue],
    written: &HashSet<u64>,
    cached: &[RawIssue],
) -> Vec<RawIssue> {
    let still_cached: HashSet<u64> = cached.iter().map(|issue| issue.number).collect();
    let survivors: Vec<RawIssue> = fetched
        .iter()
        .filter(|issue| !written.contains(&issue.number) || still_cached.contains(&issue.number))
        .cloned()
        .collect();
    let written: Vec<RawIssue> = cached
        .iter()
        .filter(|issue| written.contains(&issue.number))
        .cloned()
        .collect();
    merge_issues(&survivors, &written)
}

/// `issues` with the stamps written to the cache while their fetch was in flight. An
/// issue still exactly as it was in `start` was reused rather than read, so the
/// cache's stamp is newer, and writing the old one back would have the next sweep
/// hydrate it again. Any other issue keeps its own stamp: its detail came from this
/// fetch, and a stamp it never read would hide that detail from the next sweep.
fn with_stamps_moved_meanwhile(
    mut issues: Vec<RawIssue>,
    start: &[RawIssue],
    cached: &[RawIssue],
) -> Vec<RawIssue> {
    let by_number = |issues: &'_ [RawIssue]| -> HashMap<u64, RawIssue> {
        issues
            .iter()
            .map(|issue| (issue.number, issue.clone()))
            .collect()
    };
    let (start, cached) = (by_number(start), by_number(cached));
    for issue in &mut issues {
        if start.get(&issue.number) == Some(&*issue) {
            if let Some(newer) = cached.get(&issue.number) {
                issue.updated_at.clone_from(&newer.updated_at);
            }
        }
    }
    issues
}

/// Resolve an issue number to a [`DependencyRef`] (number + title + url) against
/// the board's identity map. Numbers absent from the board (e.g. closed or beyond
/// the fetched page) yield `None`, so they are simply not shown as a badge.
fn dependency_ref(number: u64, issue_by_number: &HashMap<u64, PrdRef>) -> Option<DependencyRef> {
    issue_by_number.get(&number).map(|prd| DependencyRef {
        number,
        title: prd.title.clone(),
        url: prd.url.clone(),
    })
}

/// Resolve a Slice's candidate blocker numbers (native or prose) into badge
/// refs, keeping only blockers still open in this board. Numbers absent from
/// `open_numbers` (closed or not in the fetched board) are dropped, avoiding a
/// false Blocked state.
fn resolve_open_blockers(
    candidates: impl IntoIterator<Item = u64>,
    open_numbers: &HashSet<u64>,
    issue_by_number: &HashMap<u64, PrdRef>,
) -> Vec<DependencyRef> {
    candidates
        .into_iter()
        .filter(|number| open_numbers.contains(number))
        .filter_map(|number| dependency_ref(number, issue_by_number))
        .collect()
}

/// Index the swept open Linked PRs by the issue numbers they close, so each
/// Slice can pick up its own badges in sweep order. A PR closing several issues
/// appears under each of them; a PR closing nothing contributes nothing here
/// and is picked up by [`unlinked_prs`] instead.
fn linked_prs_by_issue(linked_prs: &[RawLinkedPr]) -> HashMap<u64, Vec<LinkedPrRef>> {
    let mut by_issue: HashMap<u64, Vec<LinkedPrRef>> = HashMap::new();
    for linked in linked_prs {
        for issue in &linked.closes {
            by_issue.entry(*issue).or_default().push(linked.pr.clone());
        }
    }
    by_issue
}

/// The **Unlinked PRs**: the swept PRs that made it onto no Slice card, because
/// they close nothing or only close issues that are not Slices. Without this an
/// open PR the join cannot place would vanish from the board. Sweep order is
/// kept, so the most recently updated PR comes first.
fn unlinked_prs(linked_prs: &[RawLinkedPr], slices: &[Slice]) -> Vec<LinkedPrRef> {
    let on_a_card: HashSet<u64> = slices
        .iter()
        .flat_map(|slice| &slice.linked_prs)
        .map(|pr| pr.number)
        .collect();
    linked_prs
        .iter()
        .filter(|linked| !on_a_card.contains(&linked.pr.number))
        .map(|linked| linked.pr.clone())
        .collect()
}

/// The open issues that promote their children: the PRDs and Suggested PRDs of the
/// first classification pass, then every Slice reachable from them or from a Parent the
/// board does not hold. Repeated until nothing new is found, so promotion depends
/// on neither the depth of a split nor the order of the board, and a Parent cycle
/// with no plan behind it promotes nothing.
fn open_plan(
    raw_issues: &[RawIssue],
    open_numbers: &HashSet<u64>,
    open_vetoed: &HashSet<u64>,
) -> HashSet<u64> {
    let open_issues = || raw_issues.iter().filter(|raw| !raw.closed);
    let mut plan: HashSet<u64> = open_issues()
        .filter(|raw| {
            matches!(
                classify_issue(raw),
                IssueClassification::Prd | IssueClassification::SuggestedPrd
            )
        })
        .map(|raw| raw.number)
        .collect();
    loop {
        let found: Vec<u64> = open_issues()
            .filter(|raw| !plan.contains(&raw.number))
            .filter(|raw| {
                classify_on_board(raw, open_numbers, &plan, open_vetoed)
                    == IssueClassification::Slice
            })
            .map(|raw| raw.number)
            .collect();
        if found.is_empty() {
            return plan;
        }
        plan.extend(found);
    }
}

/// Pure projection from the board's two authoritative collections — the open
/// raw issues and the open Linked PRs of the last **Sweep** — to a classified
/// board.
///
/// The two are joined by issue number: a Slice's `pr #n @u` badges, its **WIP**
/// state, its **Best PR** and its Decorations all come from the PRs whose
/// closing references name it. A PR the join places on no Slice is an **Unlinked
/// PR**, and a Slice no PR closes simply has none — which is how a merged PR
/// disappears, since it is absent from the sweep (ADR 0008).
pub fn classify(raw_issues: &[RawIssue], linked_prs: &[RawLinkedPr]) -> ClassifiedBoard {
    let mut prs_by_issue = linked_prs_by_issue(linked_prs);

    // The numbers of issues that are still open in this board, so prose
    // blockers (which carry no open/closed state of their own) and native
    // blockers (which may include closed issues) can be filtered to open
    // ones only, avoiding a false Blocked state.
    let open_numbers: HashSet<u64> = raw_issues
        .iter()
        .filter(|raw| !raw.closed)
        .map(|raw| raw.number)
        .collect();

    // Identity of every issue in the board, so a Slice's parent reference
    // (native or prose) resolves to its PRD ref (number + title + url) for the
    // swimlane header and link.
    let issue_by_number: HashMap<u64, PrdRef> = raw_issues
        .iter()
        .filter(|raw| !raw.closed)
        .map(|raw| {
            (
                raw.number,
                PrdRef {
                    number: raw.number,
                    title: raw.title.clone(),
                    url: raw.url.clone(),
                    suggested: classify_issue(raw) == IssueClassification::SuggestedPrd,
                },
            )
        })
        .collect();

    let open_vetoed: HashSet<u64> = raw_issues
        .iter()
        .filter(|raw| !raw.closed && has_veto(raw))
        .map(|raw| raw.number)
        .collect();
    let open_plan = open_plan(raw_issues, &open_numbers, &open_vetoed);

    let mut slices = Vec::new();
    let mut prds = Vec::new();
    let mut other = Vec::new();

    for raw in raw_issues.iter().cloned() {
        if raw.closed {
            continue;
        }

        match classify_on_board(&raw, &open_numbers, &open_plan, &open_vetoed) {
            IssueClassification::Slice => {
                let body_str = raw.body.as_deref().unwrap_or("");
                // Use native blockers when present; otherwise fall back to
                // prose. Both feed the same open-set filter + ref resolution
                // (see `resolve_open_blockers`) for the blocker badges.
                let blockers: Vec<DependencyRef> = if !raw.native_blockers.is_empty() {
                    resolve_open_blockers(
                        raw.native_blockers.iter().copied(),
                        &open_numbers,
                        &issue_by_number,
                    )
                } else {
                    resolve_open_blockers(
                        parse_blockers_from_body(body_str),
                        &open_numbers,
                        &issue_by_number,
                    )
                };
                // Look the Parent up among the issues in this board.
                let prd = lane_parent_of(&raw, &open_plan)
                    .and_then(|number| issue_by_number.get(&number).cloned());
                slices.push(RawSlice {
                    number: raw.number,
                    title: raw.title,
                    url: raw.url,
                    closed: false,
                    prd,
                    assignee: raw.assignee,
                    assignee_avatar_url: raw.assignee_avatar_url,
                    linked_prs: prs_by_issue.remove(&raw.number).unwrap_or_default(),
                    blockers,
                    // A `## Blocked by` section naming no issue is a real wait
                    // the dependency graph cannot see. Native links are checked
                    // first: an issue GitHub already links is never "external",
                    // however its blockers resolve.
                    external_blocker: raw.native_blockers.is_empty()
                        && has_unreferenced_blocked_by(body_str),
                    labels: raw.labels,
                    // Filled by `resolve_unblocks` once the board is mapped.
                    unblocks: Vec::new(),
                });
            }
            IssueClassification::Prd => {
                prds.push(Prd {
                    number: raw.number,
                    title: raw.title,
                });
            }
            classification => {
                other.push(OtherIssue {
                    number: raw.number,
                    title: raw.title,
                    url: raw.url,
                    classification,
                });
            }
        }
    }

    // Derive the reverse "unblocks" edge across the whole board, then project
    // each raw Slice into its read model with the derived state.
    resolve_unblocks(&mut slices);
    let slices: Vec<Slice> = slices.into_iter().map(RawSlice::into_slice).collect();
    let unlinked_prs = unlinked_prs(linked_prs, &slices);

    ClassifiedBoard {
        slices,
        prds,
        other,
        unlinked_prs,
    }
}

/// The seam between the application and the OS secure store (real or fake).
///
/// `infrastructure` implements this against the operating system's credential
/// store (keyring); use-cases run against a fake in tests, so no real keyring
/// is touched.
#[async_trait]
pub trait SecureStorePort: Send + Sync {
    /// Persist the Personal Access Token, replacing any existing one.
    async fn save_token(&self, token: &GitHubToken) -> AppAction;
    /// Read the stored token, or `None` when none has been saved yet.
    async fn load_token(&self) -> AppResult<Option<GitHubToken>>;
    /// Remove the stored token, if any (signing out).
    async fn delete_token(&self) -> AppAction;
}

/// Shared stores are stores too, so the composition root can pick a store at
/// runtime (`Arc<dyn SecureStorePort>`) and hand it to an [`AuthService`].
#[async_trait]
impl<S: SecureStorePort + ?Sized> SecureStorePort for Arc<S> {
    async fn save_token(&self, token: &GitHubToken) -> AppAction {
        (**self).save_token(token).await
    }

    async fn load_token(&self) -> AppResult<Option<GitHubToken>> {
        (**self).load_token().await
    }

    async fn delete_token(&self) -> AppAction {
        (**self).delete_token().await
    }
}

/// The seam between the application and on-device persistence of which project
/// was last opened, so the app can reopen it on the next launch.
///
/// `infrastructure` implements this against a local file; use-cases run against
/// a fake in tests, so no disk is touched.
#[async_trait]
pub trait ProjectStorePort: Send + Sync {
    /// The project opened most recently, or `None` if none has been opened yet.
    async fn last_opened(&self) -> AppResult<Option<RepoRef>>;
    /// Remember `repo` as the most recently opened project.
    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction;
    /// The recent-projects list cached on the last successful fetch, or `None`
    /// when the cache is cold (nothing cached yet, or it could not be read).
    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>>;
    /// Replace the cached recent-projects list with `projects`.
    async fn cache_projects(&self, projects: &[Project]) -> AppAction;
    /// The repositories the user has summoned by name on the home screen, in
    /// newest-added-first order.
    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>>;
    /// Add a repo to the tracked list if not already present (idempotent).
    async fn track_repo(&self, repo: &RepoRef) -> AppAction;
    /// Remove a repo from the tracked list.
    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction;
    /// The persisted theme preference, or `None` when unset.
    async fn theme_preference(&self) -> AppResult<Option<ThemePreference>>;
    /// Persist the selected theme preference.
    async fn remember_theme_preference(&self, theme: ThemePreference) -> AppAction;
    /// The persisted board view mode, or `None` when unset (default: columns).
    async fn view_mode(&self) -> AppResult<Option<BoardViewMode>>;
    /// Persist the selected board view mode.
    async fn remember_view_mode(&self, mode: BoardViewMode) -> AppAction;
    /// Clear every account-scoped piece of local state — the last-opened
    /// project, the tracked-repos list, and the cached recent-projects list —
    /// in one operation, leaving the theme and view-mode preferences untouched
    /// (they are not tied to the signed-in Viewer, so they survive signing out).
    async fn clear_account_data(&self) -> AppAction;
}

/// Shared stores are stores too, so the composition root can hand the same
/// `Arc<dyn ProjectStorePort>` to a [`ProjectsService`].
#[async_trait]
impl<S: ProjectStorePort + ?Sized> ProjectStorePort for Arc<S> {
    async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        (**self).last_opened().await
    }

    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction {
        (**self).remember_last_opened(repo).await
    }

    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>> {
        (**self).cached_projects().await
    }

    async fn cache_projects(&self, projects: &[Project]) -> AppAction {
        (**self).cache_projects(projects).await
    }

    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>> {
        (**self).tracked_repos().await
    }

    async fn track_repo(&self, repo: &RepoRef) -> AppAction {
        (**self).track_repo(repo).await
    }

    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction {
        (**self).untrack_repo(repo).await
    }

    async fn theme_preference(&self) -> AppResult<Option<ThemePreference>> {
        (**self).theme_preference().await
    }

    async fn remember_theme_preference(&self, theme: ThemePreference) -> AppAction {
        (**self).remember_theme_preference(theme).await
    }

    async fn view_mode(&self) -> AppResult<Option<BoardViewMode>> {
        (**self).view_mode().await
    }

    async fn remember_view_mode(&self, mode: BoardViewMode) -> AppAction {
        (**self).remember_view_mode(mode).await
    }

    async fn clear_account_data(&self) -> AppAction {
        (**self).clear_account_data().await
    }
}

/// The seam between the application and the per-project board snapshot cache.
///
/// `infrastructure` implements this against local files (one snapshot per
/// `owner/repo`); use-cases run against an in-memory fake in tests.
#[async_trait]
pub trait BoardCachePort: Send + Sync {
    /// The cached snapshot for `repo`, or `None` when no cache exists yet.
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>>;
    /// Replace `repo`'s cached snapshot.
    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction;
    /// Cache footprint grouped by project and summed across all projects.
    async fn cache_usage(&self) -> AppResult<BoardCacheUsage>;
    /// Remove the cached snapshot for one project.
    async fn clear_board(&self, repo: &RepoRef) -> AppAction;
    /// Remove every cached board snapshot.
    async fn clear_all(&self) -> AppAction;
}

/// Shared caches are caches too, so the composition root can hand an
/// `Arc<dyn BoardCachePort>` to use-cases.
#[async_trait]
impl<C: BoardCachePort + ?Sized> BoardCachePort for Arc<C> {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        (**self).cached_board(repo).await
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
        (**self).cache_board(repo, snapshot).await
    }

    async fn cache_usage(&self) -> AppResult<BoardCacheUsage> {
        (**self).cache_usage().await
    }

    async fn clear_board(&self, repo: &RepoRef) -> AppAction {
        (**self).clear_board(repo).await
    }

    async fn clear_all(&self) -> AppAction {
        (**self).clear_all().await
    }
}

/// Use-cases for the project board.
pub struct BoardService<P: GitHubPort> {
    port: P,
}

impl<P: GitHubPort> BoardService<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    /// Assign the authenticated user to a Ready Slice, claiming it on GitHub.
    ///
    /// On success the caller re-polls the board: the now-assigned Slice derives
    /// `Wip` and leaves the Ready column. On failure the error carries the repo
    /// and issue for context and the board is left unchanged.
    pub async fn assign_self(&self, repo: &RepoRef, issue_number: u64) -> AppAction {
        self.port
            .assign_self(repo, issue_number)
            .await
            .map_err(|err| {
                err.with_context("repo", repo)
                    .with_context("issue", issue_number)
            })
    }

    /// Confirm a *suggested* classification by adding its tier-1 label (`prd` or
    /// `slice`) to the issue on GitHub.
    ///
    /// The label is derived from the suggestion via
    /// [`IssueClassification::suggested_label`]; a classification with nothing to
    /// confirm (already confident, or unclassified) is rejected as invalid input
    /// before any GitHub call. On success the caller re-polls the board: the
    /// now-labelled issue classifies tier-1 and leaves the "other open issues"
    /// bucket. On failure the error carries the repo and issue for context and
    /// the issue is left unchanged.
    pub async fn confirm_classification(
        &self,
        repo: &RepoRef,
        issue_number: u64,
        classification: &IssueClassification,
    ) -> AppAction {
        let label = classification.suggested_label().ok_or_else(|| {
            AppError::invalid_input("This issue has no suggested classification to confirm.")
                .with_context("repo", repo)
                .with_context("issue", issue_number)
        })?;

        self.port
            .add_label(repo, issue_number, label)
            .await
            .map_err(|err| {
                err.with_context("repo", repo)
                    .with_context("issue", issue_number)
                    .with_context("label", label)
            })
    }

    /// Load the project's board view plus a retained snapshot captured at fetch
    /// start, to support unchanged refresh decisions.
    ///
    /// Both collections are fetched: the open issues, and the **Sweep** of open
    /// Linked PRs the board joins onto them.
    pub async fn load(&self, repo: &RepoRef) -> AppResult<LoadedBoard> {
        let fetched_at = Utc::now();
        let raw_issues = self
            .port
            .load_issues(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))?;
        let linked_prs = self.sweep_open_prs(repo).await?;

        Ok(LoadedBoard {
            board: classify(&raw_issues, &linked_prs),
            snapshot: BoardSnapshot::new(raw_issues, linked_prs, fetched_at),
        })
    }

    /// The open-PR sweep for `repo`, the authority on which Linked PRs exist.
    /// The hot refresh path's only fetch (ADR 0008).
    ///
    /// The PRs stay in GitHub's order (most recently updated first, which the
    /// Unlinked PR list shows), but each PR's closed issues are sorted, so that
    /// [`same_open_prs`] need not care how GitHub listed them.
    async fn sweep_open_prs(&self, repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        let mut linked_prs = self
            .port
            .sweep_open_prs(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))?;
        for linked in &mut linked_prs {
            linked.closes.sort_unstable();
            linked.closes.dedup();
        }
        Ok(linked_prs)
    }

    /// **Hydrate** `numbers`: the second half of a **Cascade**, and of a structural
    /// refresh.
    async fn hydrate_issues(&self, repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        self.port
            .hydrate_issues(repo, numbers)
            .await
            .map_err(|err| {
                err.with_context("repo", repo)
                    .with_context("issues", format!("{numbers:?}"))
            })
    }

    /// Refresh against a retained snapshot and report whether the board changed.
    ///
    /// This is the **structural** refresh, and it writes only issues. The index
    /// **Sweep** says what exists: anything absent from it is dropped, with no need
    /// to ask GitHub about it, and only the numbers that are new or whose stamp moved
    /// are **Hydrated**. It joins the result with the Linked PRs the snapshot already
    /// carries, so it can never overwrite the hot path's fresher PR facts with an
    /// older list of its own (ADR 0008).
    ///
    /// The sweep and its hydrate land together or not at all, so a failed hydrate
    /// leaves every stamp unmoved and the next tick finds the same change.
    pub async fn refresh(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
    ) -> AppResult<BoardRefresh> {
        let fetched_at = Utc::now();
        let index = self
            .port
            .sweep_open_issues(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))?;
        let open: HashSet<u64> = index.iter().map(|stamp| stamp.number).collect();
        let survivors: Vec<RawIssue> = snapshot
            .raw_issues
            .iter()
            .filter(|issue| open.contains(&issue.number))
            .cloned()
            .collect();
        let stale = stale_issues(&survivors, &index);
        let raw_issues = if stale.is_empty() {
            survivors
        } else {
            let hydrated = self.hydrate_issues(repo, &stale).await?;
            merge_hydrated_issues(&survivors, &stale, &hydrated)
        };
        Ok(refresh_outcome(
            snapshot.with_issues(raw_issues, fetched_at),
            snapshot,
        ))
    }

    /// The **safety-net** refresh: re-read every open issue and replace the
    /// snapshot's issue collection wholesale, healing what GitHub did not stamp (an
    /// edited relationship). Like [`BoardService::refresh`] it writes only issues,
    /// carrying the snapshot's Linked PRs through untouched.
    async fn safety_net_refresh(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
    ) -> AppResult<BoardRefresh> {
        let fetched_at = Utc::now();
        let raw_issues = self
            .port
            .load_issues(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))?;
        Ok(refresh_outcome(
            snapshot.with_issues(raw_issues, fetched_at),
            snapshot,
        ))
    }

    /// Load and classify all open issues for a project, returning only the board
    /// view (compatibility wrapper over [`BoardService::load`]).
    pub async fn classify_board(&self, repo: &RepoRef) -> AppResult<ClassifiedBoard> {
        Ok(self.load(repo).await?.board)
    }
}

/// The first-paint source when opening a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardOpen {
    /// Warm cache: loaded instantly from local storage.
    Cached(LoadedBoard),
    /// Cold cache: loaded from GitHub and seeded locally.
    Cold(LoadedBoard),
}

/// Use-cases for opening and refreshing a board with a per-project local cache.
pub struct CachedBoardService<G: GitHubPort, C: BoardCachePort> {
    board: BoardService<G>,
    cache: C,
}

impl<G: GitHubPort, C: BoardCachePort> CachedBoardService<G, C> {
    pub fn new(github: G, cache: C) -> Self {
        Self {
            board: BoardService::new(github),
            cache,
        }
    }

    /// Open a board with stale-while-revalidate semantics:
    /// - warm cache => instant local paint;
    /// - cold cache => full load and cache seed.
    pub async fn open(&self, repo: &RepoRef) -> AppResult<BoardOpen> {
        if let Some(snapshot) = self
            .cache
            .cached_board(repo)
            .await
            .map_err(|err| err.with_operation("CachedBoardService::open"))?
        {
            return Ok(BoardOpen::Cached(LoadedBoard {
                board: classify(&snapshot.raw_issues, &snapshot.linked_prs),
                snapshot,
            }));
        }

        let loaded = self.board.load(repo).await?;
        self.cache
            .cache_board(repo, &loaded.snapshot)
            .await
            .map_err(|err| err.with_operation("CachedBoardService::open"))?;
        Ok(BoardOpen::Cold(loaded))
    }

    /// The **structural** refresh: sweep the issue index, drop what it no longer
    /// lists, hydrate what is new or moved, and rewrite the cache only when that
    /// found something. An unchanged repo hydrates nothing, repaints nothing and
    /// writes nothing.
    ///
    /// Issue-side only: whatever the hot path or a safety net wrote while this
    /// refresh was in flight is adopted before the result is returned or cached, so
    /// a slower structural write can never revert 15-second-fresh PR facts, nor
    /// bring back an issue a safety net dropped or corrected (ADR 0008).
    pub async fn structural_refresh_cached(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
    ) -> AppResult<BoardRefresh> {
        let start = self.freshest_snapshot(repo, snapshot).await?;
        let refresh = self.board.refresh(repo, &start).await?;
        let refresh = self.with_facts_written_since(repo, &start, refresh).await?;
        self.cache_issue_side_result(repo, &start, &refresh).await?;
        Ok(refresh)
    }

    /// The **hot** refresh: sweep the repo's open Pull Requests, replace the
    /// board's Linked PR collection with the result, and rewrite the cache
    /// **only** when the open-PR set actually changed — an identical sweep
    /// repaints nothing, hydrates nothing and writes nothing, unless an issue is
    /// still awaiting its close (below).
    ///
    /// This is the 15-second path of the Freshness contract, and the only writer
    /// of PR facts. It sweeps no issues, so it cannot be delayed by a structural
    /// refresh walking a large repo; it writes its sweep onto the freshest issues
    /// known rather than onto the ones its caller happened to be holding (ADR 0008).
    ///
    /// A change to which PRs close which issues also **Cascades**: those issues
    /// are hydrated in the same operation. A PR merges, its Slice reads closed,
    /// and the Slices it blocked derive **Ready** in the same repaint, with no
    /// further fetch, because `classify` derives blocker openness from the board.
    /// The sweep and its hydrate land together or not at all: writing the PRs
    /// alone would make the next sweep look unchanged and lose the cascade, so a
    /// failed hydrate fails the refresh and the next tick finds the same change.
    ///
    /// GitHub closes a merged PR's issues a moment after the PR leaves the sweep,
    /// so an issue a vanished PR leaves reading open is read **once more** on the
    /// next tick, whatever that sweep says (see `BoardSnapshot::awaiting_close`).
    /// That second read still honours "an identical sweep repaints nothing": when
    /// the issue simply stayed open, as after a PR closed without merging, only the
    /// bookkeeping is written.
    pub async fn hot_refresh_cached(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
    ) -> AppResult<BoardRefresh> {
        let linked_prs = self.board.sweep_open_prs(repo).await?;
        let freshest = self.freshest_snapshot(repo, snapshot).await?;
        // Judged against the facts it builds on, not against the caller's
        // possibly-staler base: the hot path owns the Linked PRs and nothing
        // else, so only a changed open-PR set is its news. Issue facts it
        // merely carries forward were already painted and cached by the path
        // that fetched them. Checked first because this runs every 15 seconds
        // and the snapshot it would otherwise build copies every issue.
        let same_prs = same_open_prs(&linked_prs, &freshest.linked_prs);
        if same_prs && freshest.awaiting_close.is_empty() {
            return Ok(BoardRefresh::Unchanged(freshest));
        }
        // An identical set keeps its order, which alone is not news.
        let linked_prs = if same_prs {
            freshest.linked_prs.clone()
        } else {
            linked_prs
        };
        let cascade: Vec<u64> = cascade_issues(&freshest.linked_prs, &linked_prs)
            .into_iter()
            .chain(freshest.awaiting_close.iter().copied())
            .collect::<BTreeSet<u64>>()
            .into_iter()
            .collect();
        let refreshed = if cascade.is_empty() {
            freshest.with_linked_prs(linked_prs)
        } else {
            let hydrated = self.board.hydrate_issues(repo, &cascade).await?;
            // The hydrate yielded, so the issue side may have written meanwhile.
            // An issue it wrote keeps that write: the hydrate may have read it
            // earlier, and overlaying it would undo a closure. Dropping the hydrate's
            // read loses nothing, because the next index sweep compares against
            // what is written here and finds anything still out of date.
            let current = self.freshest_snapshot(repo, &freshest).await?;
            let written = issues_changed_between(&freshest.raw_issues, &current.raw_issues);
            let cascade: Vec<u64> = cascade
                .into_iter()
                .filter(|number| !written.contains(number))
                .collect();
            let hydrated: Vec<RawIssue> = hydrated
                .into_iter()
                .filter(|issue| !written.contains(&issue.number))
                .collect();
            let mut merged = current
                .with_linked_prs(linked_prs)
                .with_hydrated_issues(&cascade, &hydrated);
            merged.awaiting_close = issues_awaiting_close(
                &freshest.linked_prs,
                &merged.linked_prs,
                freshest
                    .awaiting_close
                    .iter()
                    .copied()
                    .filter(|number| written.contains(number)),
                &merged.raw_issues,
            );
            merged
        };
        self.cache
            .cache_board(repo, &refreshed)
            .await
            .map_err(|err| err.with_operation("CachedBoardService::hot_refresh_cached"))?;
        Ok(refresh_outcome(refreshed, &freshest))
    }

    /// The freshest board facts known for `repo`: the last write any refresh
    /// path made to the cache they share, falling back to `base` when the cache
    /// holds nothing. Each path rebuilds its result on top of this before
    /// writing, so whichever finishes last keeps the other's collection instead
    /// of reverting it. A cache miss is not a failure, it only means there is
    /// nothing fresher than the caller already held; a failed read is, because
    /// the other path's newer facts may be in there and writing without them
    /// would revert them.
    ///
    /// The paths interleave only at their network awaits. From this read to
    /// the write it feeds nothing may yield, which holds because refreshes run
    /// as tasks on the single UI thread and the cache adapter's I/O never
    /// suspends. An adapter that does suspend must serialize this per repo.
    async fn freshest_snapshot(
        &self,
        repo: &RepoRef,
        base: &BoardSnapshot,
    ) -> AppResult<BoardSnapshot> {
        let cached = self
            .cache
            .cached_board(repo)
            .await
            .map_err(|err| err.with_operation("CachedBoardService::freshest_snapshot"))?;
        Ok(cached.unwrap_or_else(|| base.clone()))
    }

    /// Re-decide an issue-side refresh's outcome against everything written since
    /// the fetch began, when the cache held `start`, so its result carries the hot
    /// path's PR facts and **Cascade**, and the issues the other issue-side path
    /// wrote, rather than older ones: a structural refresh finishing after a
    /// safety net keeps what that safety net dropped or corrected. Only writes made
    /// after `start` are protected: anything earlier was there for the fetch to
    /// supersede. If the other write's read was the older one, the next index sweep
    /// finds it out of date.
    async fn with_facts_written_since(
        &self,
        repo: &RepoRef,
        start: &BoardSnapshot,
        refresh: BoardRefresh,
    ) -> AppResult<BoardRefresh> {
        let refreshed = match &refresh {
            BoardRefresh::Changed(loaded) => &loaded.snapshot,
            BoardRefresh::Unchanged(snapshot) => snapshot,
        };
        let freshest = self.freshest_snapshot(repo, start).await?;
        let written = issues_changed_between(&start.raw_issues, &freshest.raw_issues);
        let raw_issues =
            with_issues_written_meanwhile(&refreshed.raw_issues, &written, &freshest.raw_issues);
        let raw_issues =
            with_stamps_moved_meanwhile(raw_issues, &start.raw_issues, &freshest.raw_issues);
        Ok(refresh_outcome(
            BoardSnapshot {
                awaiting_close: freshest.awaiting_close,
                ..BoardSnapshot::new(raw_issues, freshest.linked_prs, refreshed.fetched_at)
            },
            start,
        ))
    }

    /// Write an issue-side result to the cache unless the cache already holds those
    /// exact facts. Stamps count here, unlike for a repaint: a moved stamp left
    /// unwritten would be hydrated again on every later sweep. `fetched_at` does not
    /// count, so an idle board never touches the disk.
    async fn cache_issue_side_result(
        &self,
        repo: &RepoRef,
        start: &BoardSnapshot,
        refresh: &BoardRefresh,
    ) -> AppAction {
        let result = match refresh {
            BoardRefresh::Changed(loaded) => &loaded.snapshot,
            BoardRefresh::Unchanged(snapshot) => snapshot,
        };
        let cached = self.freshest_snapshot(repo, start).await?;
        if result.raw_issues == cached.raw_issues && result.linked_prs == cached.linked_prs {
            return Ok(());
        }
        self.cache
            .cache_board(repo, result)
            .await
            .map_err(|err| err.with_operation("CachedBoardService::cache_issue_side_result"))
    }

    /// The **safety-net** refresh: a full **Hydrate** of every open issue, the one
    /// path that assumes GitHub lied about everything else. It heals what no
    /// stamp reports (for example, an edited native dependency), so such an edit
    /// is corrected within one safety-net interval.
    ///
    /// Issue-side only, like the structural refresh: it adopts whatever the hot
    /// path wrote meanwhile instead of writing PR facts of its own, so it never
    /// reverts them. It repaints, and rewrites the cache, only on a real
    /// difference.
    pub async fn safety_net_refresh_cached(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
    ) -> AppResult<BoardRefresh> {
        let start = self.freshest_snapshot(repo, snapshot).await?;
        let refresh = self.board.safety_net_refresh(repo, &start).await?;
        let refresh = self.with_facts_written_since(repo, &start, refresh).await?;
        self.cache_issue_side_result(repo, &start, &refresh).await?;
        Ok(refresh)
    }

    /// The manual **Refresh**: the hot, structural and safety-net passes once
    /// each, in that order, so the user's escape hatch is strictly stronger than
    /// any single loop.
    ///
    /// The passes are reported as one outcome, judged against `snapshot` (the
    /// board on screen): `Changed` if together they found anything to repaint.
    /// A failed pass writes nothing and does not stop the passes after it, so a
    /// broken issue side still refreshes PR facts, and the other way round; every
    /// failure is reported beside the outcome.
    ///
    /// `hot_pass_done` runs as soon as the hot pass has finished, so a caller that
    /// serializes hot sweeps can hold its gate for that pass alone, rather than
    /// blocking every PR update behind the slower issue passes.
    ///
    /// The outcome is the freshest board in the cache once the passes are done,
    /// not the last pass's own answer: a background refresh may have written newer
    /// facts meanwhile, notably after a failed pass, and reporting the older answer
    /// would repaint them away.
    pub async fn full_refresh_cached(
        &self,
        repo: &RepoRef,
        snapshot: &BoardSnapshot,
        hot_pass_done: impl FnOnce(),
    ) -> FullRefresh {
        fn absorb(
            pass: AppResult<BoardRefresh>,
            latest: &mut BoardSnapshot,
            failures: &mut Vec<AppError>,
        ) {
            match pass {
                Ok(BoardRefresh::Changed(loaded)) => *latest = loaded.snapshot,
                Ok(BoardRefresh::Unchanged(snapshot)) => *latest = snapshot,
                Err(error) => failures.push(error),
            }
        }

        let mut latest = snapshot.clone();
        let mut failures = Vec::new();
        let hot = self.hot_refresh_cached(repo, &latest).await;
        absorb(hot, &mut latest, &mut failures);
        hot_pass_done();
        let structural = self.structural_refresh_cached(repo, &latest).await;
        absorb(structural, &mut latest, &mut failures);
        let safety_net = self.safety_net_refresh_cached(repo, &latest).await;
        absorb(safety_net, &mut latest, &mut failures);
        match self.freshest_snapshot(repo, &latest).await {
            Ok(freshest) => latest = freshest,
            Err(error) => failures.push(error),
        }
        FullRefresh {
            refresh: refresh_outcome(latest, snapshot),
            failures,
        }
    }
}

/// Use-cases for Personal Access Token authentication, backed by a
/// [`SecureStorePort`].
pub struct AuthService<S: SecureStorePort> {
    store: S,
}

impl<S: SecureStorePort> AuthService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Validate a pasted PAT and persist it to the OS secure store, so it is
    /// reused across launches. Rejects anything that is not a fine-grained PAT
    /// before it ever reaches the store.
    pub async fn save_token(&self, raw: &str) -> AppAction {
        let token = GitHubToken::parse(raw)?;
        self.store
            .save_token(&token)
            .await
            .map_err(|err| err.with_operation("AuthService::save_token"))
    }

    /// The stored token, or an `Unauthorized` error when none is saved yet.
    ///
    /// Callers use the `Unauthorized` case to route the user to the paste-token
    /// screen.
    pub async fn require_token(&self) -> AppResult<GitHubToken> {
        self.store
            .load_token()
            .await
            .map_err(|err| err.with_operation("AuthService::require_token"))?
            .ok_or_else(|| {
                AppError::unauthorized("Add a Personal Access Token to load your board.")
                    .with_operation("AuthService::require_token")
            })
    }

    /// Whether a token is already stored, to decide the launch screen.
    pub async fn has_token(&self) -> AppResult<bool> {
        Ok(self
            .store
            .load_token()
            .await
            .map_err(|err| err.with_operation("AuthService::has_token"))?
            .is_some())
    }

    /// Remove the stored token (sign out).
    pub async fn clear_token(&self) -> AppAction {
        self.store
            .delete_token()
            .await
            .map_err(|err| err.with_operation("AuthService::clear_token"))
    }
}

/// Use-case for the signed-in account: the Viewer's identity for the account
/// menu, and signing out. Backed by a [`SecureStorePort`] (the token), a
/// [`ProjectStorePort`] (account-scoped local state), and a [`BoardCachePort`]
/// (cached board snapshots) — the three sign-out touches. The Viewer fetch
/// takes its [`GitHubPort`] per call rather than storing one, since sign-out
/// never needs it: signing out must not require wiring a live GitHub client or
/// a valid token just to clear local state.
pub struct AccountService<S: SecureStorePort, P: ProjectStorePort, C: BoardCachePort> {
    secure_store: S,
    project_store: P,
    board_cache: C,
}

impl<S: SecureStorePort, P: ProjectStorePort, C: BoardCachePort> AccountService<S, P, C> {
    pub fn new(secure_store: S, project_store: P, board_cache: C) -> Self {
        Self {
            secure_store,
            project_store,
            board_cache,
        }
    }

    /// The signed-in Viewer (login, name, avatar), for the account menu.
    pub async fn viewer<G: GitHubPort>(&self, github: &G) -> AppResult<Viewer> {
        github
            .viewer()
            .await
            .map_err(|err| err.with_operation("AccountService::viewer"))
    }

    /// Sign out: remove the stored token, clear account-scoped local state
    /// (tracked repos, last-opened project, cached recent projects), and clear
    /// every cached board snapshot — resetting the app to its first-run,
    /// no-token state. UI preferences (theme, board view mode) are
    /// deliberately left untouched, since they are not tied to the Viewer.
    ///
    /// Each step is best-effort-ordered but not best-effort-tolerant: a failure
    /// stops the sequence and surfaces immediately, so a partial sign-out is
    /// never silently swallowed.
    pub async fn sign_out(&self) -> AppAction {
        self.secure_store
            .delete_token()
            .await
            .map_err(|err| err.with_operation("AccountService::sign_out"))?;
        self.project_store
            .clear_account_data()
            .await
            .map_err(|err| err.with_operation("AccountService::sign_out"))?;
        self.board_cache
            .clear_all()
            .await
            .map_err(|err| err.with_operation("AccountService::sign_out"))
    }
}

/// Use-case for the home screen: listing the accessible projects,
/// most-recently-pushed first, backed by a [`GitHubPort`].
pub struct ProjectsService<G: GitHubPort> {
    github: G,
}

impl<G: GitHubPort> ProjectsService<G> {
    pub fn new(github: G) -> Self {
        Self { github }
    }

    /// The accessible projects, most-recently-pushed first.
    ///
    /// Ordering is owned here rather than trusted from the adapter: `pushed_at`
    /// is an RFC-3339 UTC timestamp, so a descending lexical sort puts the most
    /// recently active project first.
    pub async fn recent_projects(&self) -> AppResult<Vec<Project>> {
        let mut projects = self
            .github
            .list_projects()
            .await
            .map_err(|err| err.with_operation("ProjectsService::recent_projects"))?;
        projects.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
        Ok(projects)
    }
}

/// The outcome of a stale-while-revalidate refresh of the recent-projects list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectsRefresh {
    /// The live list differs from the cache (including a cold cache): the cache
    /// has been rewritten and the UI should swap to this list.
    Changed(Vec<Project>),
    /// The live list matches the cache: nothing was written and the UI should
    /// stay put (no flicker).
    Unchanged,
}

/// Use-case for the home screen's recent-projects list with
/// **stale-while-revalidate** caching: the cached list renders instantly while a
/// live fetch refreshes it — and the local cache — only when it actually
/// changed.
///
/// It composes the recency-sorted live fetch ([`ProjectsService`]) with the
/// on-device cache ([`ProjectStorePort`]), so the recency ordering stays owned in
/// one place and the change decision is testable against fakes (no disk, no live
/// GitHub).
pub struct RecentProjectsService<G: GitHubPort, S: ProjectStorePort> {
    projects: ProjectsService<G>,
    cache: S,
}

impl<G: GitHubPort, S: ProjectStorePort> RecentProjectsService<G, S> {
    pub fn new(github: G, cache: S) -> Self {
        Self {
            projects: ProjectsService::new(github),
            cache,
        }
    }

    /// The cached list for an instant first paint, or `None` on a cold cache.
    /// A local read only: no token or network involved.
    pub async fn cached(&self) -> AppResult<Option<Vec<Project>>> {
        self.cache
            .cached_projects()
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::cached"))
    }

    /// Fetch the live list (recency-sorted), compare it with the cache, and
    /// persist + report it only when it changed; an unchanged list leaves the
    /// cache untouched and reports [`ProjectsRefresh::Unchanged`].
    pub async fn refresh(&self) -> AppResult<ProjectsRefresh> {
        let live = self.projects.recent_projects().await?;
        let cached = self
            .cache
            .cached_projects()
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::refresh"))?;
        if cached.as_deref() == Some(live.as_slice()) {
            return Ok(ProjectsRefresh::Unchanged);
        }
        self.cache
            .cache_projects(&live)
            .await
            .map_err(|err| err.with_operation("RecentProjectsService::refresh"))?;
        Ok(ProjectsRefresh::Changed(live))
    }
}

/// Use-cases for remembering and reopening the last-opened project, backed by a
/// [`ProjectStorePort`]. A purely local concern: unlike [`ProjectsService`] it
/// needs neither a token nor the network, so it never fails for auth reasons.
pub struct LastOpenedService<S: ProjectStorePort> {
    store: S,
}

impl<S: ProjectStorePort> LastOpenedService<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Remember `repo` as the most recently opened project, so the next launch
    /// reopens it.
    pub async fn open_project(&self, repo: &RepoRef) -> AppAction {
        self.store
            .remember_last_opened(repo)
            .await
            .map_err(|err| err.with_operation("LastOpenedService::open_project"))
    }

    /// The project to reopen on launch, or `None` to show the home screen.
    pub async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        self.store
            .last_opened()
            .await
            .map_err(|err| err.with_operation("LastOpenedService::last_opened"))
    }
}

/// Use-case for opening a project summoned by name on the home screen (the
/// go-to action), composing the live board fetch ([`BoardService`]) with the
/// on-device tracked/last-opened store ([`ProjectStorePort`]).
///
/// Keeping this orchestration here — rather than in the presentation layer —
/// makes the "track only on a successful open" behaviour testable at the
/// use-case seam against fakes (no GitHub, no disk).
pub struct TrackedProjectsService<G: GitHubPort, S: ProjectStorePort> {
    board: BoardService<G>,
    store: S,
}

impl<G: GitHubPort, S: ProjectStorePort> TrackedProjectsService<G, S> {
    pub fn new(github: G, store: S) -> Self {
        Self {
            board: BoardService::new(github),
            store,
        }
    }

    /// Open a repo summoned by name: load and classify its board to verify
    /// access, then — only on success — track the repo (idempotent) and
    /// remember it as last-opened before returning the board. A failed load
    /// (e.g. a 404 for a repo that does not exist or is not accessible)
    /// propagates and tracks nothing.
    ///
    /// The track and last-opened writes are deliberately **best-effort**: the
    /// board has already loaded, so a local-store write failure must not fail an
    /// otherwise-successful open. Such a failure simply leaves the repo
    /// untracked for now — the next successful open re-attempts it — rather than
    /// surfacing an error to the user.
    pub async fn open_and_track(&self, repo: &RepoRef) -> AppResult<LoadedBoard> {
        let board = self.board.load(repo).await?;
        let _ = self.store.track_repo(repo).await;
        let _ = self.store.remember_last_opened(repo).await;
        Ok(board)
    }
}
