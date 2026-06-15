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
    // an empty body maps to `None`. Its one native blocker is CLOSED, so it is
    // dropped from `native_blockers`. An open linked PR is detected.
    let child = issue_by_number(&issues, 3);
    assert_eq!(child.native_parent, Some(1));
    assert!(child.is_native_child_of_prd);
    assert_eq!(child.body, None);
    assert_eq!(child.native_blockers, Vec::<u64>::new());
    assert!(child.has_open_linked_pr);
    assert_eq!(child.assignee.as_deref(), Some("carlos-verdes"));

    // A `slice`-labelled issue: only the OPEN native blocker (#3) is kept; the
    // CLOSED one (#2) is dropped. No native parent here (prose handles it).
    let slice = issue_by_number(&issues, 5);
    assert_eq!(slice.labels, vec!["slice".to_string()]);
    assert_eq!(slice.native_blockers, vec![3]);
    assert_eq!(slice.native_parent, None);
    assert!(!slice.has_open_linked_pr);

    // An unlabeled issue stays label-free for the heuristic tier.
    let unlabeled = issue_by_number(&issues, 9);
    assert!(unlabeled.labels.is_empty());

    // Closed issues are fetched too (so a closed native link is visible); the
    // mapping records `closed` and `classify_board` is what omits them.
    let closed = issue_by_number(&issues, 2);
    assert!(closed.closed);
    assert!(closed.is_native_child_of_prd);
}
