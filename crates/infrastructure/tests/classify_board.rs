//! Integration test: the classify-board use-case runs end-to-end against the fake port.

use application::{BoardService, ClassifiedBoard};
use async_trait::async_trait;
use domain::{
    group_into_lanes, AppAction, AppResult, IssueClassification, Project, RawIssue, RawLinkedPr,
    RepoRef,
};
use infrastructure::FakeGitHubPort;

#[tokio::test]
async fn classify_board_splits_issues_into_slices_prds_and_other() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard {
        slices,
        prds,
        other,
        ..
    } = service
        .classify_board(&repo)
        .await
        .expect("fake port should classify the board");

    // Tier-1 Slices go onto the board.
    assert!(!slices.is_empty(), "expected at least one confirmed Slice");

    // Tier-1 PRDs are collected separately.
    assert!(!prds.is_empty(), "expected at least one confirmed PRD");
    assert!(
        prds.iter().any(|p| p.number == 1),
        "issue #1 (prd label) should be a PRD"
    );

    // The closed issue must be omitted entirely.
    assert!(
        !slices.iter().any(|s| s.number == 2),
        "closed issue #2 must be omitted from slices"
    );
    assert!(
        !prds.iter().any(|p| p.number == 2),
        "closed issue #2 must be omitted from prds"
    );
    assert!(
        !other.iter().any(|o| o.number == 2),
        "closed issue #2 must be omitted from other"
    );

    // Tier-2: suggested issues appear in "other open issues".
    let suggested_prd = other.iter().find(|o| o.number == 8);
    assert!(
        suggested_prd.is_some(),
        "issue #8 (PRD headings, no label) should be in other"
    );
    assert_eq!(
        suggested_prd.unwrap().classification,
        IssueClassification::SuggestedPrd,
        "issue #8 should be classified as SuggestedPrd"
    );

    let suggested_slice = other.iter().find(|o| o.number == 9);
    assert!(
        suggested_slice.is_some(),
        "issue #9 (Slice headings, no label) should be in other"
    );
    assert_eq!(
        suggested_slice.unwrap().classification,
        IssueClassification::SuggestedSlice,
        "issue #9 should be classified as SuggestedSlice"
    );

    // Tier-3: unclassified issues appear in "other open issues".
    let unclassified = other.iter().find(|o| o.number == 10);
    assert!(
        unclassified.is_some(),
        "issue #10 (no label, no headings) should be in other"
    );
    assert_eq!(
        unclassified.unwrap().classification,
        IssueClassification::Unclassified,
        "issue #10 should be Unclassified"
    );
}

#[tokio::test]
async fn classify_board_derives_blocked_state_from_native_blockers() {
    use domain::SliceState;

    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fake port should classify the board");

    struct Case {
        issue: u64,
        expected_state: SliceState,
        expected_open_blockers: usize,
    }
    let cases = [
        Case {
            // Issue #5 carries native blockers [3, 2], but only #3 is open.
            issue: 5,
            expected_state: SliceState::Blocked,
            expected_open_blockers: 1,
        },
        Case {
            // Issue #3 carries only a CLOSED native blocker (#2), so classifier
            // filtering must drop it and avoid a false Blocked state.
            issue: 3,
            expected_state: SliceState::Wip,
            expected_open_blockers: 0,
        },
    ];

    for case in cases {
        let slice = slices
            .iter()
            .find(|s| s.number == case.issue)
            .unwrap_or_else(|| panic!("issue #{} should be a confirmed Slice", case.issue));
        assert_eq!(
            slice.state, case.expected_state,
            "issue #{} derived unexpected state from native blockers",
            case.issue
        );
        assert_eq!(
            slice.blockers.len(),
            case.expected_open_blockers,
            "issue #{} had unexpected open-blocker count after classifier filtering",
            case.issue
        );
    }
}

#[tokio::test]
async fn classify_board_joins_swept_linked_prs_onto_the_slice() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fake port should classify the board");

    // The fake sweep returns PR #12, whose closing reference names issue #3;
    // classify must join it onto that Slice for the `pr #n @u` badge.
    let slice3 = slices
        .iter()
        .find(|s| s.number == 3)
        .expect("issue #3 should be a confirmed Slice");
    assert_eq!(
        slice3.linked_prs.len(),
        1,
        "issue #3 is closed by one open PR in the sweep"
    );
    assert_eq!(slice3.linked_prs[0].number, 12);
    assert_eq!(
        slice3.linked_prs[0].author.as_deref(),
        Some("carlos-verdes")
    );
    assert_eq!(
        slice3.assignee_avatar_url.as_deref(),
        Some("https://avatars.githubusercontent.com/u/1?v=4"),
        "issue #3 assignee avatar should be carried to the Slice read model"
    );
}

#[tokio::test]
async fn classify_board_resolves_prd_title_from_native_and_prose_parents() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fake port should classify the board");

    // Issue #3 links its parent natively to PRD #1, so its card is tagged with
    // that PRD's title.
    let slice3 = slices
        .iter()
        .find(|s| s.number == 3)
        .expect("issue #3 should be a confirmed Slice");
    assert_eq!(
        slice3.prd.as_ref().map(|prd| prd.title.as_str()),
        Some("Zfirot desktop dashboard"),
        "issue #3's native parent should resolve to PRD #1's title"
    );

    // Issue #5 has no native parent but a prose "## Parent" pointing at #1, so
    // the prose fallback resolves the same PRD title.
    let slice5 = slices
        .iter()
        .find(|s| s.number == 5)
        .expect("issue #5 should be a confirmed Slice");
    assert_eq!(
        slice5.prd.as_ref().map(|prd| prd.title.as_str()),
        Some("Zfirot desktop dashboard"),
        "issue #5's prose parent should resolve to PRD #1's title"
    );
}

#[derive(Clone)]
struct ClosedParentFixturePort;

#[async_trait]
impl application::GitHubPort for ClosedParentFixturePort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![
            RawIssue {
                number: 50,
                title: "Closed PRD".to_string(),
                url: "https://github.com/funkode-io/zfirot/issues/50".to_string(),
                body: None,
                labels: vec!["prd".to_string()],
                closed: true,
                native_parent: None,
                native_blockers: vec![],
                assignee: None,
                assignee_avatar_url: None,
                is_native_child_of_prd: false,
            },
            RawIssue {
                number: 51,
                title: "Open Slice with closed native parent".to_string(),
                url: "https://github.com/funkode-io/zfirot/issues/51".to_string(),
                body: None,
                labels: vec!["slice".to_string()],
                closed: false,
                native_parent: Some(50),
                native_blockers: vec![],
                assignee: None,
                assignee_avatar_url: None,
                is_native_child_of_prd: false,
            },
        ])
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(vec![])
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

#[tokio::test]
async fn classify_board_places_slice_with_closed_native_parent_in_no_prd_lane() {
    let service = BoardService::new(ClosedParentFixturePort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fixture port should classify the board");

    assert_eq!(slices.len(), 1, "fixture should return one open slice");
    assert_eq!(
        slices[0].prd, None,
        "a slice whose native parent PRD is closed must render under No PRD"
    );
}

/// Board fixture for the **Blocked reason** rules: every Slice below is tier-1
/// (`slice` label) so this test exercises the reasons alone, not promotion.
#[derive(Clone)]
struct BlockedReasonFixturePort;

fn raw_slice_issue(number: u64, body: &str, labels: &[&str], closed: bool) -> RawIssue {
    RawIssue {
        number,
        title: format!("Slice {number}"),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: Some(body.to_string()),
        labels: labels.iter().map(|l| l.to_string()).collect(),
        closed,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        is_native_child_of_prd: false,
    }
}

#[async_trait]
impl application::GitHubPort for BlockedReasonFixturePort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![
            // Waiting on a prod environment that is not an issue (dxp-data-loom#1250).
            raw_slice_issue(
                100,
                "## What to build\n\nShip it.\n\n## Blocked by\n\n- A prod values file existing for this app.\n",
                &["slice"],
                false,
            ),
            // The same, plus the sticky labels: three reasons at once.
            raw_slice_issue(
                101,
                "## Blocked by\n\nWaiting on the platform team.\n",
                &["slice", "deferred", "blocked", "severity:low"],
                false,
            ),
            // A maintainer must decide; nothing else says so.
            raw_slice_issue(102, "Plain body.", &["slice", "needs-triage"], false),
            // Names a blocker that has since closed: workable again, not an
            // External blocker.
            raw_slice_issue(
                103,
                "## Blocked by\n\n- #104\n",
                &["slice"],
                false,
            ),
            raw_slice_issue(104, "Done and dusted.", &["slice"], true),
            // No signal at all.
            raw_slice_issue(105, "Plain body.", &["slice", "enhancement"], false),
            // The "no blockers" placeholder the planning skills emit for every
            // unblocked Slice: it asserts the opposite of a wait, so it must
            // not read as an External blocker.
            raw_slice_issue(
                106,
                "## What to build\n\nShip it.\n\n## Blocked by\n\n- None — can start immediately.\n",
                &["slice"],
                false,
            ),
        ])
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(vec![])
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

/// The board keeps Ready meaning "an Agent can take this now": a Slice waiting
/// on anything at all leaves Ready and says why.
#[tokio::test]
async fn classify_board_derives_blocked_reasons_and_keeps_them_out_of_ready() {
    use domain::{BlockedReason, SliceState};

    let service = BoardService::new(BlockedReasonFixturePort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fixture port should classify the board");

    struct Case {
        issue: u64,
        expected_state: SliceState,
        expected_reasons: Vec<BlockedReason>,
    }
    let cases = [
        Case {
            issue: 100,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![BlockedReason::ExternalBlocker],
        },
        Case {
            issue: 101,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![
                BlockedReason::ExternalBlocker,
                BlockedReason::Deferred,
                BlockedReason::LabelledBlocked,
            ],
        },
        Case {
            issue: 102,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![BlockedReason::NeedsTriage],
        },
        Case {
            issue: 103,
            expected_state: SliceState::Ready,
            expected_reasons: vec![],
        },
        Case {
            issue: 105,
            expected_state: SliceState::Ready,
            expected_reasons: vec![],
        },
        Case {
            issue: 106,
            expected_state: SliceState::Ready,
            expected_reasons: vec![],
        },
    ];

    for case in cases {
        let slice = slices
            .iter()
            .find(|s| s.number == case.issue)
            .unwrap_or_else(|| panic!("issue #{} should be a Slice", case.issue));
        assert_eq!(
            slice.state, case.expected_state,
            "issue #{} landed in the wrong column",
            case.issue
        );
        assert_eq!(
            slice.blocked_reasons, case.expected_reasons,
            "issue #{} derived the wrong reasons",
            case.issue
        );
    }

    // A Blocked Slice can have no blocker issues at all — the card must cope.
    let external = slices.iter().find(|s| s.number == 100).unwrap();
    assert!(
        external.blockers.is_empty(),
        "an External blocker names no issue, so there are no blocker badges"
    );

    // Closed Slices are Done: hidden from the board entirely, whatever reason
    // labels they carry.
    assert!(
        !slices.iter().any(|s| s.number == 104),
        "a closed Slice stays Done and off the board"
    );
}

/// Board fixture for "a Parent makes a Slice". Nothing here is tier-1 evidence
/// except the PRD's own label and the two control Slices, so every other issue
/// reaches a Lane by its Parent or not at all.
#[derive(Clone)]
struct PromotionFixturePort;

const PROMOTION_PRD: u64 = 200;

fn native_parented_issue(number: u64, native_parent: u64, body: &str) -> RawIssue {
    RawIssue {
        native_parent: Some(native_parent),
        ..raw_slice_issue(number, body, &[], false)
    }
}

#[async_trait]
impl application::GitHubPort for PromotionFixturePort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![
            raw_slice_issue(PROMOTION_PRD, "The plan.", &["prd"], false),
            raw_slice_issue(201, "A bug report.", &["bug"], false),
            raw_slice_issue(202, "A closed plan.", &["prd"], true),
            // Promoted by a prose Parent alone.
            raw_slice_issue(210, "## Parent\n\n#200\n", &[], false),
            // Promoted despite carrying only non-classifying labels.
            raw_slice_issue(211, "## Parent\n\n#200\n", &["deferred", "ci"], false),
            // dxp-data-loom#1250: split off a sibling, deferred and waiting on
            // something that is not an issue.
            raw_slice_issue(
                212,
                "## Parent\n\n#200\n\n## Blocked by\n\n- A prod values file existing for this app.\n",
                &["deferred", "blocked"],
                false,
            ),
            // Blocked by an open sibling, reached only through prose.
            raw_slice_issue(213, "## Parent\n\n#200\n\n## Blocked by\n\n- #210\n", &[], false),
            // Native link wins, in both directions.
            native_parented_issue(220, PROMOTION_PRD, "## Parent\n\n#201\n"),
            native_parented_issue(221, 201, "## Parent\n\n#200\n"),
            // A parent that is not a PRD promotes nothing, with or without a
            // heading that suggests a Slice.
            raw_slice_issue(230, "## Parent\n\n#201\n", &[], false),
            raw_slice_issue(
                231,
                "## What to build\n\nA fix.\n\n## Parent\n\n#201\n",
                &[],
                false,
            ),
            // A closed or absent parent: planned work with nowhere to go.
            raw_slice_issue(240, "## Parent\n\n#202\n", &[], false),
            raw_slice_issue(241, "## Parent\n\n#999\n", &["deferred"], false),
            native_parented_issue(242, 999, ""),
            // Genuine orphans, including a Parent section naming nothing.
            raw_slice_issue(250, "Just a thought.", &[], false),
            raw_slice_issue(251, "## Parent\n\nSee the main issue.\n", &[], false),
            raw_slice_issue(
                252,
                "## What to build\n\nA fix.\n\n## Acceptance criteria\n\n- [ ] Done\n",
                &[],
                false,
            ),
            // Tier-1 classification stays as it was: a labelled Slice stays a Slice
            // whatever its Parent is, and a labelled PRD with a Parent stays a PRD.
            raw_slice_issue(260, "## Parent\n\n#201\n", &["slice"], false),
            raw_slice_issue(261, "## Parent\n\n#200\n", &["prd"], false),
        ])
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(vec![])
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

async fn promotion_board() -> ClassifiedBoard {
    BoardService::new(PromotionFixturePort)
        .classify_board(&RepoRef::new("funkode-io", "zfirot"))
        .await
        .expect("fixture port should classify the board")
}

/// Every issue whose Parent resolves lands on the right Lane, and one whose
/// Parent cannot be resolved lands in "No PRD" rather than disappearing.
#[tokio::test]
async fn classify_board_promotes_parented_issues_onto_their_lane() {
    let ClassifiedBoard { slices, .. } = promotion_board().await;

    let cases: [(u64, Option<u64>, &str); 9] = [
        (210, Some(PROMOTION_PRD), "unlabelled, prose Parent"),
        (211, Some(PROMOTION_PRD), "non-classifying labels only"),
        (212, Some(PROMOTION_PRD), "the dxp-data-loom#1250 shape"),
        (213, Some(PROMOTION_PRD), "prose Parent and prose blocker"),
        (220, Some(PROMOTION_PRD), "native PRD beats prose non-PRD"),
        (240, None, "closed Parent"),
        (241, None, "Parent outside the board"),
        (242, None, "native Parent outside the board"),
        (
            260,
            None,
            "tier-1 Slice's prose non-PRD Parent heads no Lane",
        ),
    ];

    for (issue, lane, name) in cases {
        let slice = slices
            .iter()
            .find(|s| s.number == issue)
            .unwrap_or_else(|| panic!("#{issue} ({name}) should be a Slice"));
        assert_eq!(
            slice.prd.as_ref().map(|prd| prd.number),
            lane,
            "#{issue} ({name}) is on the wrong Lane"
        );
    }
}

/// A Parent that resolves to anything but a PRD promotes nothing, so the tray
/// holds exactly those issues and the genuine orphans.
#[tokio::test]
async fn classify_board_keeps_non_prd_children_and_orphans_in_the_tray() {
    let ClassifiedBoard {
        slices,
        prds,
        other,
        ..
    } = promotion_board().await;

    let tray: Vec<u64> = other.iter().map(|issue| issue.number).collect();
    assert_eq!(
        tray,
        vec![201, 221, 230, 231, 250, 251, 252],
        "the tray should shrink to non-PRD children and genuine orphans"
    );

    let suggested = other.iter().find(|issue| issue.number == 231).unwrap();
    assert_eq!(
        suggested.classification,
        IssueClassification::SuggestedSlice,
        "an unpromoted issue keeps its first-pass classification"
    );
    assert!(
        !slices.iter().any(|s| tray.contains(&s.number)),
        "no tray issue may also be a Slice"
    );
    assert!(
        prds.iter().any(|p| p.number == 261),
        "a labelled PRD with a Parent stays a PRD"
    );
}

/// Promotion only decides *that* an issue is a Slice: its state and Blocked
/// reasons come from the same rules as a labelled Slice's.
#[tokio::test]
async fn classify_board_derives_promoted_slice_state_like_a_labelled_one() {
    use domain::{BlockedReason, SliceState};

    let ClassifiedBoard { slices, .. } = promotion_board().await;

    struct Case {
        issue: u64,
        expected_state: SliceState,
        expected_reasons: Vec<BlockedReason>,
        expected_blockers: Vec<u64>,
    }
    let cases = [
        Case {
            issue: 210,
            expected_state: SliceState::Ready,
            expected_reasons: vec![],
            expected_blockers: vec![],
        },
        Case {
            issue: 211,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![BlockedReason::Deferred],
            expected_blockers: vec![],
        },
        Case {
            issue: 212,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![
                BlockedReason::ExternalBlocker,
                BlockedReason::Deferred,
                BlockedReason::LabelledBlocked,
            ],
            expected_blockers: vec![],
        },
        Case {
            issue: 213,
            expected_state: SliceState::Blocked,
            expected_reasons: vec![BlockedReason::Dependency],
            expected_blockers: vec![210],
        },
    ];

    for case in cases {
        let slice = slices
            .iter()
            .find(|s| s.number == case.issue)
            .unwrap_or_else(|| panic!("#{} should be a Slice", case.issue));
        assert_eq!(slice.state, case.expected_state, "#{} state", case.issue);
        assert_eq!(
            slice.blocked_reasons, case.expected_reasons,
            "#{} reasons",
            case.issue
        );
        let blockers: Vec<u64> = slice.blockers.iter().map(|b| b.number).collect();
        assert_eq!(blockers, case.expected_blockers, "#{} blockers", case.issue);
    }
}

/// Board fixture for **Suggested PRD** Lanes: canned issues, so a test can edit a
/// PRD's body between two reads of the "same" board.
#[derive(Clone)]
struct IssuesPort(Vec<RawIssue>);

#[async_trait]
impl application::GitHubPort for IssuesPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(self.0.clone())
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(vec![])
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

const SUGGESTED_PRD: u64 = 300;
const CONFIDENT_PRD: u64 = 310;
const PRD_TEMPLATE: &str = "## Problem Statement\n\nX.\n\n## User Stories\n\n1. Y.\n";

/// One unlabelled PRD and one labelled PRD with the same family of children, and
/// a bug whose sub-tasks must stay off the board. `suggested_prd_body` is what
/// the unlabelled PRD's body currently says.
fn suggested_prd_issues(suggested_prd_body: &str) -> Vec<RawIssue> {
    vec![
        raw_slice_issue(SUGGESTED_PRD, suggested_prd_body, &[], false),
        raw_slice_issue(CONFIDENT_PRD, "The plan.", &["prd"], false),
        raw_slice_issue(320, "A bug report.", &["bug"], false),
        // Children of the Suggested PRD, by every route to it.
        raw_slice_issue(301, "## Parent\n\n#300\n", &[], false),
        raw_slice_issue(302, "## Parent\n\n#300\n", &["ready-for-agent"], false),
        native_parented_issue(303, SUGGESTED_PRD, ""),
        RawIssue {
            labels: vec!["slice".to_string()],
            ..native_parented_issue(304, SUGGESTED_PRD, "")
        },
        // Children of the confident PRD.
        raw_slice_issue(311, "## Parent\n\n#310\n", &[], false),
        native_parented_issue(312, CONFIDENT_PRD, ""),
        // A Parent that is neither kind of PRD promotes nothing.
        raw_slice_issue(321, "## Parent\n\n#320\n", &[], false),
    ]
}

async fn classify_issues(issues: Vec<RawIssue>) -> ClassifiedBoard {
    BoardService::new(IssuesPort(issues))
        .classify_board(&RepoRef::new("funkode-io", "zfirot"))
        .await
        .expect("fixture port should classify the board")
}

/// `(lane's PRD number, whether its header is unconfirmed, its Slices)`, in board
/// order, as the board would lay them out.
fn lane_shape(board: &ClassifiedBoard) -> Vec<(Option<u64>, bool, Vec<u64>)> {
    group_into_lanes(board.slices.clone())
        .into_iter()
        .map(|lane| {
            (
                lane.prd.as_ref().map(|prd| prd.number),
                lane.prd.is_some_and(|prd| prd.suggested),
                lane.slices.iter().map(|slice| slice.number).collect(),
            )
        })
        .collect()
}

/// An issue with the PRD headings and no label heads a Lane marked unconfirmed,
/// its children reach it by the same rules as a labelled PRD's, and the labelled
/// PRD's own Lane is untouched. Confident Lanes come first, though the suggested
/// PRD was fetched first.
#[tokio::test]
async fn classify_board_heads_a_lane_with_a_suggested_prd() {
    let board = classify_issues(suggested_prd_issues(PRD_TEMPLATE)).await;

    assert_eq!(
        lane_shape(&board),
        vec![
            (Some(CONFIDENT_PRD), false, vec![311, 312]),
            (Some(SUGGESTED_PRD), true, vec![301, 302, 303, 304]),
        ]
    );
}

/// A Parent that is neither a PRD nor a Suggested PRD still promotes nothing, and
/// a Suggested PRD with no active Slices has no Lane, so like any PRD-shaped
/// suggestion it stays in the tray with its Confirm action.
#[tokio::test]
async fn classify_board_promotes_only_against_prds_and_suggested_prds() {
    let board = classify_issues(suggested_prd_issues(PRD_TEMPLATE)).await;

    let tray: Vec<(u64, IssueClassification)> = board
        .other
        .iter()
        .map(|issue| (issue.number, issue.classification.clone()))
        .collect();
    assert_eq!(
        tray,
        vec![
            (SUGGESTED_PRD, IssueClassification::SuggestedPrd),
            (320, IssueClassification::Unclassified),
            (321, IssueClassification::Unclassified),
        ]
    );

    let childless = classify_issues(vec![raw_slice_issue(400, PRD_TEMPLATE, &[], false)]).await;
    assert!(
        lane_shape(&childless).is_empty(),
        "an empty Lane is omitted, as for a confident PRD"
    );
    assert_eq!(
        childless.other[0].classification,
        IssueClassification::SuggestedPrd
    );
}

/// Editing the body out of the template drops the Lane. Its prose-parented Slices
/// go with it: an unlabelled one back to the tray, a labelled one to "No PRD",
/// since a label still makes it a Slice. A labelled Slice with a native link keeps
/// its Lane, now unmarked because nothing says its head is a PRD. An unlabelled
/// native child looks exactly like a bug's sub-task once the heading evidence is
/// gone, so it leaves too.
#[tokio::test]
async fn classify_board_drops_a_suggested_lane_whose_body_no_longer_matches() {
    let board = classify_issues(suggested_prd_issues("## Summary\n\nReworded.\n")).await;

    assert_eq!(
        lane_shape(&board),
        vec![
            (Some(SUGGESTED_PRD), false, vec![304]),
            (Some(CONFIDENT_PRD), false, vec![311, 312]),
            (None, false, vec![302]),
        ]
    );

    let tray: Vec<u64> = board.other.iter().map(|issue| issue.number).collect();
    assert_eq!(tray, vec![SUGGESTED_PRD, 320, 301, 303, 321]);

    // With no natively linked Slice to hold it open, the Lane is gone outright.
    let issues = vec![
        raw_slice_issue(SUGGESTED_PRD, "## Summary\n\nReworded.\n", &[], false),
        raw_slice_issue(301, "## Parent\n\n#300\n", &[], false),
    ];
    assert!(lane_shape(&classify_issues(issues).await).is_empty());
}

/// Both passes read only their input, so the board does not depend on the order
/// GitHub lists issues in beyond first-seen ordering: a child listed before its
/// Suggested PRD still lands on its Lane.
#[tokio::test]
async fn classify_board_promotes_a_child_listed_before_its_suggested_prd() {
    let mut issues = suggested_prd_issues(PRD_TEMPLATE);
    issues.reverse();

    let board = classify_issues(issues.clone()).await;

    let lanes = lane_shape(&board);
    assert!(
        lanes
            .iter()
            .any(|(prd, marked, slices)| *prd == Some(SUGGESTED_PRD)
                && *marked
                && slices == &vec![304, 303, 302, 301]),
        "unexpected lanes: {lanes:?}"
    );
    assert_eq!(
        board,
        classify_issues(issues).await,
        "the same facts classify to the same board"
    );
}
