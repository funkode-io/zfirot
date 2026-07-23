//! The GraphQL payload-to-`RawIssue` projection for the two-tier classifier,
//! tested offline against a recorded response fixture
//! (`tests/fixtures/issues.json`). This pins the live `load_issues` mapping —
//! the HTTP call is exercised manually, but every field the classifier reads is
//! asserted here so the projection cannot silently drift.

use domain::RawIssue;
use infrastructure::parse_issues_response;

const ISSUES_FIXTURE: &str = include_str!("fixtures/issues.json");

fn mapped_issues() -> Vec<RawIssue> {
    let (issues, next) = parse_issues_response(ISSUES_FIXTURE).expect("fixture should parse");
    assert_eq!(next, None, "the fixture is a single, final page");
    issues
}

fn issue_by_number(issues: &[RawIssue], number: u64) -> &RawIssue {
    issues
        .iter()
        .find(|issue| issue.number == number)
        .unwrap_or_else(|| panic!("no issue #{number} in the mapped fixture"))
}

#[test]
fn maps_labels_state_and_native_links_into_raw_issues() {
    let issues = mapped_issues();
    assert_eq!(issues.len(), 5);

    // The PRD: `prd` label, open, no parent, body carried through.
    let prd = issue_by_number(&issues, 1);
    assert_eq!(prd.labels, vec!["prd".to_string()]);
    assert!(!prd.closed);
    assert_eq!(prd.native_parent, None);
    assert!(!prd.is_native_child_of_prd);
    assert!(prd.body.is_some());

    // A native child of the PRD: parent number resolved, parent is `prd`, and
    // an empty body maps to `None`. Native blockers are carried through as-is
    // (including CLOSED blockers) so classifier-level filtering has all facts.
    // Two open linked PRs are lifted, one with a resolved author and one with a
    // null author (no login).
    let child = issue_by_number(&issues, 3);
    assert_eq!(child.native_parent, Some(1));
    assert!(child.is_native_child_of_prd);
    assert_eq!(child.body, None);
    assert_eq!(child.native_blockers, vec![2]);
    assert_eq!(child.linked_prs.len(), 2);
    assert_eq!(child.linked_prs[0].number, 12);
    assert_eq!(child.linked_prs[0].author.as_deref(), Some("carlos-verdes"));
    assert_eq!(child.linked_prs[0].title, "Implement SliceState derivation");
    assert_eq!(
        child.linked_prs[0].url,
        "https://github.com/funkode-io/zfirot/pull/12"
    );
    // The approved PR (isDraft=false, reviewDecision=APPROVED) derives Approved.
    assert_eq!(child.linked_prs[0].pr_status, domain::PrStatus::Approved);
    // MERGEABLE -> no Conflicts decoration.
    assert!(!child.linked_prs[0].conflicts);
    // statusCheckRollup=SUCCESS -> no CI-failing decoration.
    assert!(!child.linked_prs[0].ci_failing);
    // One of two review threads is unresolved -> count of 1 (non-blocking).
    assert_eq!(child.linked_prs[0].unresolved_comment_count, 1);
    assert_eq!(child.linked_prs[1].number, 13);
    assert_eq!(child.linked_prs[1].author, None);
    // The draft follow-up PR (isDraft=true) derives Draft regardless of review.
    assert_eq!(child.linked_prs[1].pr_status, domain::PrStatus::Draft);
    // CONFLICTING -> Conflicts decoration is set.
    assert!(child.linked_prs[1].conflicts);
    // statusCheckRollup=FAILURE -> CI-failing decoration is set.
    assert!(child.linked_prs[1].ci_failing);
    // No review threads -> no unresolved comments.
    assert_eq!(child.linked_prs[1].unresolved_comment_count, 0);
    assert_eq!(child.assignee.as_deref(), Some("carlos-verdes"));
    assert_eq!(
        child.assignee_avatar_url.as_deref(),
        Some("https://avatars.githubusercontent.com/u/1?v=4")
    );

    // A `slice`-labelled issue: both OPEN and CLOSED native blockers are kept
    // for classifier-level open-set filtering. No native parent here (prose
    // handles it).
    let slice = issue_by_number(&issues, 5);
    assert_eq!(slice.labels, vec!["slice".to_string()]);
    assert_eq!(slice.native_blockers, vec![3, 2]);
    assert_eq!(slice.native_parent, None);
    assert!(slice.linked_prs.is_empty());

    // An unlabeled issue stays label-free for the heuristic tier.
    let unlabeled = issue_by_number(&issues, 9);
    assert!(unlabeled.labels.is_empty());

    // Closed issues are fetched too (so a closed native link is visible); the
    // mapping records `closed` and `classify_board` is what omits them.
    let closed = issue_by_number(&issues, 2);
    assert!(closed.closed);
    assert!(closed.is_native_child_of_prd);
}
