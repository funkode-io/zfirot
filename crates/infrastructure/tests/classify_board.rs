//! Integration test: the classify-board use-case runs end-to-end against the fake port.

use application::{BoardService, ClassifiedBoard};
use async_trait::async_trait;
use domain::{AppAction, AppResult, IssueClassification, Project, RawIssue, RawLinkedPr, RepoRef};
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
