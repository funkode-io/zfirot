//! Integration test: the classify-board use-case runs end-to-end against the fake port.

use application::{BoardService, ClassifiedBoard};
use domain::{IssueClassification, RepoRef};
use infrastructure::FakeGitHubPort;

#[tokio::test]
async fn classify_board_splits_issues_into_slices_prds_and_other() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let ClassifiedBoard {
        slices,
        prds,
        other,
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

    // Issue #5 carries native_blockers=[3] (issue #3 is still open), so the
    // derived Slice is Blocked regardless of any prose "## Blocked by" section.
    let slice5 = slices
        .iter()
        .find(|s| s.number == 5)
        .expect("issue #5 should be a confirmed Slice");
    assert_eq!(
        slice5.state,
        SliceState::Blocked,
        "issue #5 should be Blocked from its native blocker on the still-open #3"
    );

    // Issue #3 has no native blockers and only a prose "## Blocked by - #2",
    // but #2 is closed, so the prose blocker is filtered out and #3 is not
    // falsely marked Blocked (it is WIP via its open linked PR / assignee).
    let slice3 = slices
        .iter()
        .find(|s| s.number == 3)
        .expect("issue #3 should be a confirmed Slice");
    assert_ne!(
        slice3.state,
        SliceState::Blocked,
        "issue #3's only prose blocker (#2) is closed, so it must not be Blocked"
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
