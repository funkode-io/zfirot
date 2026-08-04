use serde::{Deserialize, Serialize};

use crate::PrdRef;

/// Why a Blocked Slice cannot be picked up.
///
/// Ranked by how much the reason says about *what must change*, weakest first,
/// so a Slice's primary reason is the `max` of the reasons that apply — the same
/// shape as [`crate::PrStatus`] and `best_pr`. Only [`BlockedReason::Dependency`]
/// is self-updating (it disappears when the blocking issue closes); the rest are
/// human-authored and sticky, which is why the weakest of them is the bare
/// `blocked` label. See ADR 0006.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BlockedReason {
    /// Carries the `blocked` label and gives no other evidence.
    LabelledBlocked,
    /// Carries the `deferred` label — deliberately put off.
    Deferred,
    /// Carries the `needs-triage` label — a maintainer must decide.
    NeedsTriage,
    /// Has a `## Blocked by` section that names no issue at all, so the wait is
    /// on something the dependency graph cannot see.
    ExternalBlocker,
    /// Has at least one open "blocked by" dependency.
    Dependency,
}

impl BlockedReason {
    /// The sticky, label-borne reason a label carries, if any. The label
    /// vocabulary lives here beside the rest of the classification vocabulary
    /// (`prd`, `slice`, `ready-for-agent`).
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "needs-triage" => Some(BlockedReason::NeedsTriage),
            "deferred" => Some(BlockedReason::Deferred),
            "blocked" => Some(BlockedReason::LabelledBlocked),
            _ => None,
        }
    }
}

/// The derived state of a [`Slice`].
///
/// Precedence among active states is Blocked > WIP > Ready. `Done` (a closed
/// Slice) is a real state too, so the derivation is total; the board simply
/// omits it from its columns ([`SliceState::BOARD`]), keeping the data around so
/// Done Slices can be shown later if needed. The state is a pure derivation over
/// current GitHub data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SliceState {
    /// Blockers all closed, no open linked PR, and no assignee.
    Ready,
    /// An open Pull Request is linked to the Slice, or an assignee has claimed
    /// it.
    Wip,
    /// At least one open "blocked by" dependency.
    Blocked,
    /// A closed Slice. Hidden from the active board.
    Done,
}

impl SliceState {
    /// Board column order, left to right. `Done` is intentionally excluded so
    /// closed Slices are hidden from the active board.
    pub const BOARD: [SliceState; 3] = [SliceState::Ready, SliceState::Wip, SliceState::Blocked];
}

/// At-a-glance counts of the active board's Slices by state, for the summary
/// strip above the columns.
///
/// Only the three board states are counted; `Done` (closed) Slices are hidden
/// from the active board, so they are excluded from every count — `total` is
/// therefore `ready + wip + blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BoardSummary {
    pub ready: usize,
    pub wip: usize,
    pub blocked: usize,
}

impl BoardSummary {
    /// Count the Slices in each board state, ignoring `Done` Slices.
    ///
    /// A pure derivation over the Slices' already-derived [`SliceState`], so it
    /// stays consistent with what the columns show.
    pub fn from_slices<'a>(slices: impl IntoIterator<Item = &'a Slice>) -> Self {
        let mut summary = BoardSummary::default();
        for slice in slices {
            match slice.state {
                SliceState::Ready => summary.ready += 1,
                SliceState::Wip => summary.wip += 1,
                SliceState::Blocked => summary.blocked += 1,
                SliceState::Done => {}
            }
        }
        summary
    }

    /// The number of Slices shown on the active board (Ready + WIP + Blocked).
    pub fn total(&self) -> usize {
        self.ready + self.wip + self.blocked
    }
}

/// A reference to a related issue, for rendering a clickable dependency badge on
/// a card: either a **blocker** (an issue this Slice is blocked by) or an issue
/// this Slice **unblocks** (the reverse edge). Carries the issue number (shown
/// on the badge), its title (shown as a tooltip), and its URL (the badge links
/// to it on GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRef {
    /// The referenced GitHub issue number.
    pub number: u64,
    /// The referenced issue's title, shown as the badge tooltip.
    pub title: String,
    /// The referenced issue's URL on GitHub, for opening it in a browser.
    pub url: String,
}

/// A reference to an open Pull Request that closes a Slice's issue (GitHub's
/// closing reference), for rendering a clickable `pr #n @u` badge on a card.
/// Carries the PR number (shown on the badge), its author's login (shown as the
/// `@u` segment, absent when GitHub cannot resolve an author), its title (shown
/// as a tooltip), and its URL (the badge links to it on GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedPrRef {
    /// The referenced GitHub Pull Request number.
    pub number: u64,
    /// The PR author's login, shown as the `@u` segment; `None` when GitHub
    /// cannot resolve an author (e.g. a deleted account), in which case the
    /// badge omits the `@u` segment.
    pub author: Option<String>,
    /// The PR's title, shown as the badge tooltip.
    pub title: String,
    /// The PR's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// The review-lifecycle stage of this PR (Draft ... Approved), derived from
    /// GitHub's draft flag and review decision. Drives the Slice's WIP headline
    /// (via its Best PR); merge-health Decorations ride on top of it.
    pub pr_status: crate::PrStatus,
    /// `true` when the PR conflicts with its base branch and needs a manual
    /// conflict merge (GitHub `mergeable = CONFLICTING`). A branch merely behind
    /// its base (auto-updatable) is deliberately not flagged.
    pub conflicts: bool,
    /// `true` when the PR's checks have settled on a failure (GitHub
    /// `statusCheckRollup = FAILURE | ERROR`). Pending checks are transient and
    /// not flagged.
    pub ci_failing: bool,
    /// Count of the PR's review threads still open (`isResolved = false`).
    /// Non-blocking — surfaced so follow-up notes are visible — and does not
    /// affect whether the PR reads as ready to merge.
    pub unresolved_comment_count: u32,
}

/// A Linked PR exactly as an open-PR **Sweep** reports it: the PR's own facts
/// plus the numbers of the issues it closes (GitHub's closing references).
///
/// This is the raw read model of the PR side of the board. It is *the*
/// authority on which PRs are open — a merged or closed PR is simply absent
/// from the next sweep — and the join onto Slices happens at classification
/// time, by issue number (see ADR 0008). A `closes` number the board does not
/// hold (a closed issue, or one outside the fetched set) contributes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawLinkedPr {
    /// The PR's facts, as rendered on a Slice's `pr #n @u` badge.
    pub pr: LinkedPrRef,
    /// The issue numbers this PR closes, from its closing references.
    pub closes: Vec<u64>,
}

impl LinkedPrRef {
    /// Whether this PR reads as **ready to merge** — Approved with no blocking
    /// Decorations (not conflicting, CI not failing). Unresolved comments do
    /// **not** disqualify it, since they do not block a merge. Derived at read
    /// time and never stored (see ADR 0004).
    pub fn is_ready_to_merge(&self) -> bool {
        self.pr_status == crate::PrStatus::Approved && !self.conflicts && !self.ci_failing
    }
}

/// A read model of a GitHub issue that is a Slice of a PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// The PRD this Slice belongs to, when known.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// Avatar URL of the assignee, when assigned and available.
    pub assignee_avatar_url: Option<String>,
    pub state: SliceState,
    /// Why this Slice is not workable, strongest reason first; empty unless the
    /// Slice is [`SliceState::Blocked`]. The first is shown as the card's chip
    /// and orders the Blocked column; the rest are surfaced on hover.
    pub blocked_reasons: Vec<BlockedReason>,
    /// The still-open issues this Slice is blocked by, for the blocker badges.
    pub blockers: Vec<DependencyRef>,
    /// The issues this Slice unblocks (the reverse "blocked by" edge), for the
    /// unblocks badges. Derived across the board by [`resolve_unblocks`].
    pub unblocks: Vec<DependencyRef>,
    /// The open Pull Requests linked to this Slice via their closing reference,
    /// for the `pr #n @u` badges.
    pub linked_prs: Vec<LinkedPrRef>,
}

impl Slice {
    /// The strongest reason this Slice is not workable — its chip, and its
    /// position in the Blocked column. `None` when the Slice is workable.
    pub fn primary_blocked_reason(&self) -> Option<BlockedReason> {
        self.blocked_reasons.first().copied()
    }

    /// The **Best PR** — the open Linked PR with the highest [`PrStatus`], whose
    /// status and Decorations drive the Slice's WIP headline. `None` when the
    /// Slice has no open PR. On a tie the later PR in input order wins, which is
    /// display-immaterial since the statuses are equal.
    pub fn best_pr(&self) -> Option<&LinkedPrRef> {
        self.linked_prs.iter().max_by_key(|pr| pr.pr_status)
    }
}

/// Raw, GitHub-shaped facts about a single issue, before its [`SliceState`] is
/// derived. An adapter projects this from GitHub (still fake for this slice);
/// the pure derivation lives in the domain so it stays testable and offline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSlice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// `true` when the issue is closed; a closed Slice is Done and hidden.
    pub closed: bool,
    /// The PRD this Slice belongs to, when known.
    pub prd: Option<PrdRef>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// Avatar URL of the assignee, when assigned and available.
    pub assignee_avatar_url: Option<String>,
    /// The open Pull Requests linked to the issue via their closing reference.
    /// A non-empty list makes the Slice WIP.
    pub linked_prs: Vec<LinkedPrRef>,
    /// The still-open "blocked by" dependencies, with their references. A
    /// non-empty list makes the Slice Blocked.
    pub blockers: Vec<DependencyRef>,
    /// `true` when the issue has a `## Blocked by` section that names no issue
    /// at all — a real wait on something outside GitHub's dependency graph.
    pub external_blocker: bool,
    /// The issue's labels, from which the sticky Blocked reasons are read.
    pub labels: Vec<String>,
    /// The issues this Slice unblocks, derived across the board by
    /// [`resolve_unblocks`]; empty until then.
    pub unblocks: Vec<DependencyRef>,
}

impl RawSlice {
    /// Project this raw issue into a [`Slice`] with its derived [`SliceState`].
    pub fn into_slice(self) -> Slice {
        let blocked_reasons = self.derive_blocked_reasons();
        let state = self.derive_state(&blocked_reasons);
        Slice {
            number: self.number,
            title: self.title,
            url: self.url,
            prd: self.prd,
            assignee: self.assignee,
            assignee_avatar_url: self.assignee_avatar_url,
            state,
            blocked_reasons,
            blockers: self.blockers,
            unblocks: self.unblocks,
            linked_prs: self.linked_prs,
        }
    }

    /// Every reason this Slice is not workable, strongest first.
    ///
    /// Pure, and total: an empty result *is* the statement that the Slice is
    /// workable, which is what [`RawSlice::derive_state`] reads.
    fn derive_blocked_reasons(&self) -> Vec<BlockedReason> {
        let mut reasons = Vec::new();
        if !self.blockers.is_empty() {
            reasons.push(BlockedReason::Dependency);
        }
        if self.external_blocker {
            reasons.push(BlockedReason::ExternalBlocker);
        }
        for label in &self.labels {
            if let Some(reason) = BlockedReason::from_label(label) {
                reasons.push(reason);
            }
        }
        // Strongest first, so the head is the chip and the column's sort key.
        reasons.sort_unstable_by(|a, b| b.cmp(a));
        reasons.dedup();
        reasons
    }

    /// The pure `SliceState` derivation.
    ///
    /// A closed Slice is always `Done`. Otherwise precedence is
    /// Blocked > WIP > Ready:
    /// - **Blocked**: not workable for any reason (see [`BlockedReason`]).
    /// - **WIP**: an open linked PR, or an assignee has claimed it to start work
    ///   (an assigned Slice is by definition no longer Ready).
    /// - **Ready**: workable, no open linked PR, and no assignee.
    fn derive_state(&self, blocked_reasons: &[BlockedReason]) -> SliceState {
        if self.closed {
            return SliceState::Done;
        }
        if !blocked_reasons.is_empty() {
            return SliceState::Blocked;
        }
        if !self.linked_prs.is_empty() || self.assignee.is_some() {
            return SliceState::Wip;
        }
        SliceState::Ready
    }
}

/// Order the Blocked column so it reads top-down as "soonest to free up":
/// strongest [`BlockedReason`] first, so `Dependency` Slices (which free
/// themselves when their blocker closes) lead and the ones awaiting a human
/// decision trail. The sort is **stable**, so Slices sharing a reason keep the
/// board's own order.
///
/// A pure derivation over already-derived reasons, so the column's order and
/// each card's chip can never disagree — they read the same rank.
pub fn order_blocked_column(slices: &mut [Slice]) {
    slices.sort_by_key(|slice| std::cmp::Reverse(slice.primary_blocked_reason()));
}

/// Derive each Slice's reverse **"unblocks"** edge from the blocker edges across
/// the whole board: a Slice unblocks every Slice that lists it as a blocker.
///
/// Pure and order-preserving — a Slice's `unblocks` list follows board input
/// order. Only fetched Slices contribute edges, so references to issues outside
/// the fetched set (e.g. closed or absent blockers) are naturally omitted.
pub fn resolve_unblocks(slices: &mut [RawSlice]) {
    use std::collections::HashMap;

    // For each blocker issue number, the references of the Slices it unblocks
    // (i.e. the Slices that listed it as a blocker), in board order.
    let mut unblocks_by_number: HashMap<u64, Vec<DependencyRef>> = HashMap::new();
    for slice in slices.iter() {
        let dependent = DependencyRef {
            number: slice.number,
            title: slice.title.clone(),
            url: slice.url.clone(),
        };
        for blocker in &slice.blockers {
            unblocks_by_number
                .entry(blocker.number)
                .or_default()
                .push(dependent.clone());
        }
    }
    for slice in slices.iter_mut() {
        // Always reset from the current blocker edges (defaulting to empty) so
        // the derivation is pure: re-running it never leaves stale reverse-edge
        // data on a Slice that no longer unblocks anything.
        slice.unblocks = unblocks_by_number.remove(&slice.number).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw Slice with no blockers, no PR, and no assignee (a Ready baseline).
    fn ready_raw() -> RawSlice {
        RawSlice {
            number: 1,
            title: "A Slice".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/1".to_string(),
            closed: false,
            prd: Some(PrdRef {
                number: 7,
                title: "A PRD".to_string(),
                url: "https://github.com/funkode-io/zfirot/issues/7".to_string(),
            }),
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            blockers: vec![],
            external_blocker: false,
            labels: vec![],
            unblocks: vec![],
        }
    }

    /// A single open linked PR reference, for exercising WIP derivation and
    /// carry-through.
    fn linked_pr() -> LinkedPrRef {
        LinkedPrRef {
            number: 200,
            author: Some("hubot".to_string()),
            title: "Implement the Slice".to_string(),
            url: "https://github.com/funkode-io/zfirot/pull/200".to_string(),
            pr_status: crate::PrStatus::AwaitingReview,
            conflicts: false,
            ci_failing: false,
            unresolved_comment_count: 0,
        }
    }

    #[test]
    fn ready_to_merge_is_approved_with_no_blocking_decorations() {
        use crate::PrStatus;
        struct Case {
            name: &'static str,
            status: PrStatus,
            conflicts: bool,
            ci_failing: bool,
            unresolved: u32,
            expected: bool,
        }
        let cases = [
            Case {
                name: "approved, clean -> ready",
                status: PrStatus::Approved,
                conflicts: false,
                ci_failing: false,
                unresolved: 0,
                expected: true,
            },
            Case {
                name: "approved with unresolved comments -> still ready (non-blocking)",
                status: PrStatus::Approved,
                conflicts: false,
                ci_failing: false,
                unresolved: 3,
                expected: true,
            },
            Case {
                name: "approved but conflicting -> not ready",
                status: PrStatus::Approved,
                conflicts: true,
                ci_failing: false,
                unresolved: 0,
                expected: false,
            },
            Case {
                name: "approved but CI failing -> not ready",
                status: PrStatus::Approved,
                conflicts: false,
                ci_failing: true,
                unresolved: 0,
                expected: false,
            },
            Case {
                name: "not yet approved -> not ready",
                status: PrStatus::ChangesRequested,
                conflicts: false,
                ci_failing: false,
                unresolved: 0,
                expected: false,
            },
        ];
        for case in cases {
            let pr = LinkedPrRef {
                pr_status: case.status,
                conflicts: case.conflicts,
                ci_failing: case.ci_failing,
                unresolved_comment_count: case.unresolved,
                ..linked_pr()
            };
            assert_eq!(pr.is_ready_to_merge(), case.expected, "{}", case.name);
        }
    }

    fn pr_with_status(number: u64, status: crate::PrStatus) -> LinkedPrRef {
        LinkedPrRef {
            number,
            pr_status: status,
            ..linked_pr()
        }
    }

    #[test]
    fn best_pr_is_the_highest_status_open_pr() {
        use crate::PrStatus;
        let make = |prs| {
            RawSlice {
                linked_prs: prs,
                ..ready_raw()
            }
            .into_slice()
        };

        // No open PR -> no Best PR.
        assert!(make(vec![]).best_pr().is_none());

        // A single PR is trivially the Best PR.
        let single = make(vec![pr_with_status(1, PrStatus::Draft)]);
        assert_eq!(single.best_pr().map(|pr| pr.number), Some(1));

        // With several, the highest status wins regardless of input order.
        let many = make(vec![
            pr_with_status(1, PrStatus::Draft),
            pr_with_status(2, PrStatus::Approved),
            pr_with_status(3, PrStatus::AwaitingReview),
        ]);
        assert_eq!(
            many.best_pr().map(|pr| (pr.number, pr.pr_status)),
            Some((2, PrStatus::Approved))
        );
    }

    /// `n` distinct open blocker references, for exercising state derivation.
    fn blockers(n: u64) -> Vec<DependencyRef> {
        (0..n)
            .map(|i| DependencyRef {
                number: 100 + i,
                title: format!("Blocker {}", 100 + i),
                url: format!("https://github.com/funkode-io/zfirot/issues/{}", 100 + i),
            })
            .collect()
    }

    #[test]
    fn an_unreferenced_blocked_by_section_blocks_the_slice() {
        let raw = RawSlice {
            external_blocker: true,
            ..ready_raw()
        };

        let slice = raw.into_slice();

        assert_eq!(
            slice.state,
            SliceState::Blocked,
            "a Slice waiting on something that is not an issue is not workable"
        );
        assert_eq!(
            slice.blocked_reasons,
            vec![BlockedReason::ExternalBlocker],
            "and it says so"
        );
    }

    #[test]
    fn derives_every_blocked_reason_and_ranks_them() {
        struct Case {
            name: &'static str,
            blocker_count: u64,
            external_blocker: bool,
            labels: &'static [&'static str],
            expected: Vec<BlockedReason>,
        }
        let cases = [
            Case {
                name: "no signal at all -> workable",
                blocker_count: 0,
                external_blocker: false,
                labels: &[],
                expected: vec![],
            },
            Case {
                name: "an unrelated label is not a signal",
                blocker_count: 0,
                external_blocker: false,
                labels: &["enhancement", "layer:domain"],
                expected: vec![],
            },
            Case {
                name: "open dependency",
                blocker_count: 1,
                external_blocker: false,
                labels: &[],
                expected: vec![BlockedReason::Dependency],
            },
            Case {
                name: "needs-triage label",
                blocker_count: 0,
                external_blocker: false,
                labels: &["needs-triage"],
                expected: vec![BlockedReason::NeedsTriage],
            },
            Case {
                name: "deferred label",
                blocker_count: 0,
                external_blocker: false,
                labels: &["deferred"],
                expected: vec![BlockedReason::Deferred],
            },
            Case {
                name: "bare blocked label",
                blocker_count: 0,
                external_blocker: false,
                labels: &["blocked"],
                expected: vec![BlockedReason::LabelledBlocked],
            },
            Case {
                // dxp-data-loom#1250: deferred + blocked labels and a prose
                // `## Blocked by` naming no issue. The most informative reason
                // leads; the rest stay for the tooltip.
                name: "several signals rank strongest first",
                blocker_count: 0,
                external_blocker: true,
                labels: &["deferred", "blocked", "severity:low"],
                expected: vec![
                    BlockedReason::ExternalBlocker,
                    BlockedReason::Deferred,
                    BlockedReason::LabelledBlocked,
                ],
            },
            Case {
                name: "an open dependency outranks every sticky signal",
                blocker_count: 2,
                external_blocker: true,
                labels: &["needs-triage", "deferred", "blocked"],
                expected: vec![
                    BlockedReason::Dependency,
                    BlockedReason::ExternalBlocker,
                    BlockedReason::NeedsTriage,
                    BlockedReason::Deferred,
                    BlockedReason::LabelledBlocked,
                ],
            },
        ];

        for case in cases {
            let slice = RawSlice {
                blockers: blockers(case.blocker_count),
                external_blocker: case.external_blocker,
                labels: case.labels.iter().map(|l| l.to_string()).collect(),
                ..ready_raw()
            }
            .into_slice();

            assert_eq!(slice.blocked_reasons, case.expected, "{}", case.name);
            assert_eq!(
                slice.primary_blocked_reason(),
                case.expected.first().copied(),
                "{} — the chip is the strongest reason",
                case.name
            );
        }
    }

    #[test]
    fn derives_each_state_including_done() {
        struct Case {
            name: &'static str,
            closed: bool,
            assignee: Option<&'static str>,
            has_open_linked_pr: bool,
            open_blocker_count: u64,
            labels: &'static [&'static str],
            expected: SliceState,
        }
        let cases = [
            Case {
                name: "no blockers, no PR, no assignee -> Ready",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &[],
                expected: SliceState::Ready,
            },
            Case {
                name: "open linked PR -> WIP",
                closed: false,
                assignee: None,
                has_open_linked_pr: true,
                open_blocker_count: 0,
                labels: &[],
                expected: SliceState::Wip,
            },
            Case {
                name: "assigned but no PR -> WIP (no longer Ready)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &[],
                expected: SliceState::Wip,
            },
            Case {
                name: "open blocker -> Blocked",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 1,
                labels: &[],
                expected: SliceState::Blocked,
            },
            Case {
                name: "Blocked outranks WIP (PR + open blocker)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: true,
                open_blocker_count: 2,
                labels: &[],
                expected: SliceState::Blocked,
            },
            Case {
                name: "WIP outranks Ready (assignee present)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &[],
                expected: SliceState::Wip,
            },
            Case {
                name: "closed -> Done",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &[],
                expected: SliceState::Done,
            },
            Case {
                name: "closed wins even with an open blocker",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 3,
                labels: &[],
                expected: SliceState::Done,
            },
            Case {
                name: "a deferred label alone -> Blocked",
                closed: false,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &["deferred"],
                expected: SliceState::Blocked,
            },
            Case {
                name: "a sticky reason still outranks WIP (flat precedence, ADR 0006)",
                closed: false,
                assignee: Some("octocat"),
                has_open_linked_pr: true,
                open_blocker_count: 0,
                labels: &["needs-triage"],
                expected: SliceState::Blocked,
            },
            Case {
                name: "closed wins over a reason label — Done stays hidden",
                closed: true,
                assignee: None,
                has_open_linked_pr: false,
                open_blocker_count: 0,
                labels: &["deferred", "blocked"],
                expected: SliceState::Done,
            },
        ];

        for case in cases {
            let raw = RawSlice {
                closed: case.closed,
                assignee: case.assignee.map(str::to_string),
                linked_prs: if case.has_open_linked_pr {
                    vec![linked_pr()]
                } else {
                    vec![]
                },
                blockers: blockers(case.open_blocker_count),
                labels: case.labels.iter().map(|l| l.to_string()).collect(),
                ..ready_raw()
            };

            assert_eq!(raw.into_slice().state, case.expected, "{}", case.name);
        }
    }

    #[test]
    fn done_is_excluded_from_the_board_columns() {
        assert!(
            !SliceState::BOARD.contains(&SliceState::Done),
            "Done must not be a board column"
        );
    }

    #[test]
    fn into_slice_carries_fields_and_derived_state() {
        let raw = RawSlice {
            number: 42,
            title: "Wire the thing".to_string(),
            assignee: Some("octocat".to_string()),
            assignee_avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            linked_prs: vec![linked_pr()],
            ..ready_raw()
        };

        let slice = raw.into_slice();

        assert_eq!(slice.number, 42);
        assert_eq!(slice.title, "Wire the thing");
        assert_eq!(slice.url, "https://github.com/funkode-io/zfirot/issues/1");
        assert_eq!(
            slice.prd.as_ref().map(|prd| prd.title.as_str()),
            Some("A PRD")
        );
        assert_eq!(slice.assignee.as_deref(), Some("octocat"));
        assert_eq!(
            slice.assignee_avatar_url.as_deref(),
            Some("https://avatars.githubusercontent.com/u/1?v=4")
        );
        assert_eq!(slice.state, SliceState::Wip);
        assert_eq!(slice.linked_prs, vec![linked_pr()]);
    }

    #[test]
    fn into_slice_is_done_for_a_closed_issue() {
        let raw = RawSlice {
            closed: true,
            ..ready_raw()
        };

        assert_eq!(raw.into_slice().state, SliceState::Done);
    }

    /// A blocker reference helper for the reverse-edge tests.
    fn dep(number: u64) -> DependencyRef {
        DependencyRef {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        }
    }

    fn raw_with(number: u64, blockers: Vec<DependencyRef>) -> RawSlice {
        RawSlice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            blockers,
            ..ready_raw()
        }
    }

    /// `resolve_unblocks` fills each Slice's reverse edge from the board's
    /// blocker edges, in board order, and omits references to issues outside the
    /// fetched set.
    #[test]
    fn resolve_unblocks_derives_the_reverse_edge_across_the_board() {
        // #4 blocks #6; #6 blocks #9; #9 also lists #99 (absent from the board)
        // as a blocker, which must not produce any reverse edge.
        let mut board = vec![
            raw_with(4, vec![]),
            raw_with(6, vec![dep(4)]),
            raw_with(9, vec![dep(6), dep(99)]),
        ];

        resolve_unblocks(&mut board);

        let unblocks = |number: u64| -> Vec<u64> {
            board
                .iter()
                .find(|s| s.number == number)
                .unwrap()
                .unblocks
                .iter()
                .map(|d| d.number)
                .collect()
        };

        // #4 unblocks #6; #6 unblocks #9; #9 unblocks nothing.
        assert_eq!(unblocks(4), vec![6]);
        assert_eq!(unblocks(6), vec![9]);
        assert_eq!(unblocks(9), Vec::<u64>::new());
        // The absent blocker #99 produced no reverse edge.
        assert!(board.iter().all(|s| s.number != 99));
    }

    /// A Slice blocking several others collects all of them, in board order,
    /// each carrying the dependent's number and url for the badge link.
    #[test]
    fn resolve_unblocks_collects_all_dependents_in_order() {
        let mut board = vec![
            raw_with(1, vec![]),
            raw_with(5, vec![dep(1)]),
            raw_with(3, vec![dep(1)]),
        ];

        resolve_unblocks(&mut board);

        let one = board.iter().find(|s| s.number == 1).unwrap();
        assert_eq!(
            one.unblocks,
            vec![dep(5), dep(3)],
            "dependents follow board input order with their refs"
        );
    }

    /// `resolve_unblocks` is pure: re-running it after the blocker edges change
    /// resets each Slice's reverse edge rather than leaving stale data behind.
    #[test]
    fn resolve_unblocks_clears_stale_reverse_edges_on_rerun() {
        // First pass: #6 lists #4 as a blocker, so #4 unblocks #6.
        let mut board = vec![raw_with(4, vec![]), raw_with(6, vec![dep(4)])];
        resolve_unblocks(&mut board);
        let four = board.iter().find(|s| s.number == 4).unwrap();
        assert_eq!(four.unblocks, vec![dep(6)]);

        // The blocker edge is removed; re-running must clear #4's reverse edge.
        board
            .iter_mut()
            .find(|s| s.number == 6)
            .unwrap()
            .blockers
            .clear();
        resolve_unblocks(&mut board);

        let four = board.iter().find(|s| s.number == 4).unwrap();
        assert!(
            four.unblocks.is_empty(),
            "a Slice that no longer blocks anything has no stale reverse edge"
        );
    }

    /// A Slice in a given state, for exercising the summary counts.
    fn slice_in(number: u64, state: SliceState) -> Slice {
        Slice {
            number,
            title: format!("Slice {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            prd: None,
            assignee: None,
            assignee_avatar_url: None,
            state,
            blocked_reasons: vec![],
            blockers: vec![],
            unblocks: vec![],
            linked_prs: vec![],
        }
    }

    /// The Blocked column reads top-down as "soonest to free up": Slices whose
    /// blocker will clear itself first, the ones needing a human decision last.
    /// Within one reason the board's own order is preserved.
    #[test]
    fn order_blocked_column_sorts_by_reason_rank_and_is_stable() {
        let blocked_with = |number: u64, reasons: Vec<BlockedReason>| Slice {
            blocked_reasons: reasons,
            ..slice_in(number, SliceState::Blocked)
        };

        let mut column = vec![
            blocked_with(1, vec![BlockedReason::Deferred]),
            blocked_with(2, vec![BlockedReason::Dependency]),
            blocked_with(3, vec![BlockedReason::LabelledBlocked]),
            blocked_with(4, vec![BlockedReason::Deferred]),
            blocked_with(5, vec![BlockedReason::ExternalBlocker]),
            blocked_with(6, vec![BlockedReason::NeedsTriage]),
        ];

        order_blocked_column(&mut column);

        assert_eq!(
            column.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![2, 5, 6, 1, 4, 3],
            "ranked strongest first, with #1 before #4 since both are Deferred"
        );
    }

    #[test]
    fn board_summary_counts_each_state_and_ignores_done() {
        let slices = vec![
            slice_in(1, SliceState::Ready),
            slice_in(2, SliceState::Ready),
            slice_in(3, SliceState::Wip),
            slice_in(4, SliceState::Blocked),
            slice_in(5, SliceState::Blocked),
            slice_in(6, SliceState::Blocked),
            // Done Slices are hidden from the board, so they count for nothing.
            slice_in(7, SliceState::Done),
            slice_in(8, SliceState::Done),
        ];

        let summary = BoardSummary::from_slices(&slices);

        assert_eq!(summary.ready, 2, "two Ready Slices");
        assert_eq!(summary.wip, 1, "one WIP Slice");
        assert_eq!(summary.blocked, 3, "three Blocked Slices");
        assert_eq!(
            summary.total(),
            6,
            "the total is the visible board only (Done excluded)"
        );
    }

    #[test]
    fn board_summary_of_an_empty_board_is_all_zero() {
        let summary = BoardSummary::from_slices(&[]);

        assert_eq!(summary, BoardSummary::default());
        assert_eq!(summary.total(), 0);
    }
}
