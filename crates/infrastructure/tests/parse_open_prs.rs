//! The GraphQL payload-to-[`RawLinkedPr`] projection of the open-PR **Sweep**,
//! tested offline against a recorded response fixture
//! (`tests/fixtures/open_pull_requests.json`).
//!
//! The sweep is the board's only source of Linked PR facts (ADR 0008), so every
//! field a badge, the WIP state, the Best PR, or a Decoration reads is asserted
//! here — the HTTP call itself is exercised manually.

use application::classify;
use domain::{PrStatus, RawIssue, RawLinkedPr};
use infrastructure::parse_open_prs_response;

const OPEN_PRS_FIXTURE: &str = include_str!("fixtures/open_pull_requests.json");

fn swept_prs() -> Vec<RawLinkedPr> {
    let (prs, next) = parse_open_prs_response(OPEN_PRS_FIXTURE).expect("fixture should parse");
    assert_eq!(next, None, "the fixture is a single, final page");
    prs
}

fn pr_by_number(prs: &[RawLinkedPr], number: u64) -> &RawLinkedPr {
    prs.iter()
        .find(|linked| linked.pr.number == number)
        .unwrap_or_else(|| panic!("no PR #{number} in the swept fixture"))
}

#[test]
fn maps_every_pr_fact_a_linked_pr_badge_reads() {
    let prs = swept_prs();
    assert_eq!(prs.len(), 3);

    // An approved, mergeable PR with green checks and one unresolved thread.
    let approved = pr_by_number(&prs, 12);
    assert_eq!(approved.pr.author.as_deref(), Some("carlos-verdes"));
    assert_eq!(approved.pr.title, "Implement SliceState derivation");
    assert_eq!(
        approved.pr.url,
        "https://github.com/funkode-io/zfirot/pull/12"
    );
    assert_eq!(approved.pr.pr_status, PrStatus::Approved);
    assert!(!approved.pr.conflicts, "MERGEABLE carries no Conflicts");
    assert!(!approved.pr.ci_failing, "SUCCESS carries no CI failure");
    assert_eq!(
        approved.pr.unresolved_comment_count, 1,
        "one of two review threads is unresolved"
    );

    // A draft with a null author, a conflict and red checks.
    let draft = pr_by_number(&prs, 13);
    assert_eq!(draft.pr.author, None, "a null author leaves off the @u");
    assert_eq!(draft.pr.pr_status, PrStatus::Draft);
    assert!(draft.pr.conflicts, "CONFLICTING sets the Conflicts marker");
    assert!(draft.pr.ci_failing, "FAILURE sets the CI-failing marker");
    assert_eq!(draft.pr.unresolved_comment_count, 0);

    // A PR awaiting its first review, with no check rollup at all.
    let awaiting = pr_by_number(&prs, 14);
    assert_eq!(awaiting.pr.pr_status, PrStatus::AwaitingReview);
    assert!(!awaiting.pr.ci_failing, "no rollup is not a failure");
}

#[test]
fn carries_the_issue_numbers_each_pr_closes() {
    let prs = swept_prs();

    assert_eq!(pr_by_number(&prs, 12).closes, vec![3]);
    assert_eq!(
        pr_by_number(&prs, 13).closes,
        vec![3, 5],
        "a PR closing several issues carries all of them"
    );
    assert!(
        pr_by_number(&prs, 14).closes.is_empty(),
        "a PR that closes nothing lands on no Slice"
    );
}

#[test]
fn reports_the_next_page_cursor_while_more_prs_remain() {
    let body = r#"{ "data": { "repository": { "pullRequests": {
        "pageInfo": { "hasNextPage": true, "endCursor": "CURSOR" },
        "nodes": []
    } } } }"#;

    let (prs, next) = parse_open_prs_response(body).expect("a clean response should parse");

    assert!(prs.is_empty());
    assert_eq!(
        next.as_deref(),
        Some("CURSOR"),
        "a repo with many open PRs pages the sweep"
    );
}

#[test]
fn tolerates_partial_field_errors_beside_the_data() {
    // A token without permission to read a repo's checks gets the PR data plus
    // a field-level FORBIDDEN on `statusCheckRollup`. The sweep must still land
    // rather than failing the whole board.
    let body = r#"{
        "data": { "repository": { "pullRequests": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [ {
                "number": 9, "url": "https://x/pull/9", "title": "PR", "author": null,
                "isDraft": false, "reviewDecision": "APPROVED", "mergeable": "MERGEABLE",
                "commits": { "nodes": [ { "commit": { "statusCheckRollup": null } } ] },
                "reviewThreads": { "nodes": [] },
                "closingIssuesReferences": { "nodes": [ { "number": 1 } ] }
            } ]
        } } },
        "errors": [ { "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" } ]
    }"#;

    let (prs, _) =
        parse_open_prs_response(body).expect("partial field errors must not fail the sweep");

    assert_eq!(prs.len(), 1);
    assert!(
        !prs[0].pr.ci_failing,
        "an unreadable rollup is not a failure"
    );
    assert_eq!(prs[0].closes, vec![1]);
}

#[test]
fn fails_when_there_is_no_repository_data() {
    let body = r#"{ "data": { "repository": null },
        "errors": [ { "type": "NOT_FOUND", "message": "Could not resolve to a repository" } ] }"#;

    let error = parse_open_prs_response(body).expect_err("an absent repository must fail");

    assert_eq!(error.kind(), domain::AppErrorKind::NotFound);
}

/// One page of the sweep holding a single PR with the given body and closing
/// references, as GitHub reports it.
fn page_with_pr(number: u64, body: &str, closes: &[u64]) -> String {
    serde_json::json!({ "data": { "repository": { "pullRequests": {
        "pageInfo": { "hasNextPage": false, "endCursor": null },
        "nodes": [ {
            "number": number,
            "url": format!("https://github.com/funkode-io/zfirot/pull/{number}"),
            "title": format!("PR {number}"),
            "body": body,
            "author": null,
            "isDraft": false,
            "reviewDecision": null,
            "mergeable": "MERGEABLE",
            "commits": { "nodes": [] },
            "reviewThreads": { "nodes": [] },
            "closingIssuesReferences": {
                "nodes": closes.iter().map(|n| serde_json::json!({ "number": n })).collect::<Vec<_>>()
            }
        } ]
    } } } })
    .to_string()
}

fn slice_issue(number: u64) -> RawIssue {
    RawIssue {
        number,
        title: format!("Slice {number}"),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        is_native_child_of_prd: false,
    }
}

#[test]
fn a_pr_only_related_to_an_open_slice_shows_on_its_card() {
    let (prs, _) = parse_open_prs_response(&page_with_pr(90, "Related to #1", &[]))
        .expect("a clean response should parse");

    let board = classify(&[slice_issue(1)], &prs);

    let on_card: Vec<u64> = board.slices[0]
        .linked_prs
        .iter()
        .map(|pr| pr.number)
        .collect();
    assert_eq!(on_card, vec![90]);
    assert!(board.unlinked_prs.is_empty());
}

#[test]
fn a_pr_only_related_to_something_else_stays_unlinked() {
    let (prs, _) = parse_open_prs_response(&page_with_pr(90, "Related to #404", &[]))
        .expect("a clean response should parse");

    let board = classify(&[slice_issue(1)], &prs);

    assert!(board.slices[0].linked_prs.is_empty());
    let unlinked: Vec<u64> = board.unlinked_prs.iter().map(|pr| pr.number).collect();
    assert_eq!(unlinked, vec![90]);
}

#[test]
fn a_closing_reference_outranks_a_related_mention() {
    let (prs, _) = parse_open_prs_response(&page_with_pr(90, "Part of #2", &[1]))
        .expect("a clean response should parse");

    assert_eq!(
        prs[0].closes,
        vec![1],
        "a mention must not claim a second Slice from a PR that names its own"
    );
}

#[test]
fn a_prs_title_and_body_relate_to_issues_independently() {
    let cases = [
        (
            "a relation ending the title",
            "fix: update refs",
            "#12 reproduces it",
            vec![],
        ),
        (
            "a not ending the title",
            "Support SQL NOT",
            "Related to #12",
            vec![12],
        ),
        ("a relation in the title", "Related to #12", "", vec![12]),
    ];

    for (name, title, body, expected) in cases {
        let page = page_with_pr(90, body, &[]).replace("PR 90", title);

        let (prs, _) = parse_open_prs_response(&page).expect("a clean response should parse");

        assert_eq!(prs[0].closes, expected, "{name}");
    }
}
