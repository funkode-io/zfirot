//! Integration test: the classify-board use-case runs end-to-end against the fake port.

use application::{BoardService, ClassifiedBoard};
use async_trait::async_trait;
use domain::{AgentRef, AppAction, AppResult, IssueClassification, Project, RawIssue, RepoRef};
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
async fn classify_board_carries_linked_prs_onto_the_slice() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard { slices, .. } = service
        .classify_board(&repo)
        .await
        .expect("fake port should classify the board");

    // Issue #3 carries an open linked PR in the fake data; classify_board must
    // copy it through onto the rendered Slice for the `pr #n @u` badge.
    let slice3 = slices
        .iter()
        .find(|s| s.number == 3)
        .expect("issue #3 should be a confirmed Slice");
    assert_eq!(
        slice3.linked_prs.len(),
        1,
        "issue #3 has one open linked PR"
    );
    assert_eq!(slice3.linked_prs[0].number, 12);
    assert_eq!(
        slice3.linked_prs[0].author.as_deref(),
        Some("carlos-verdes")
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
                linked_prs: vec![],
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
                linked_prs: vec![],
                is_native_child_of_prd: false,
            },
        ])
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
        Ok(vec![])
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
