//! The issue-index **Sweep**'s parser. Its answer is authoritative, so whatever it
//! leaves out is dropped from the board: every way of reading short has to fail
//! rather than read as "those issues are gone".

use domain::{AppErrorKind, IssueStamp};
use infrastructure::parse_issue_index_response;

fn page(has_next: bool, cursor: &str, nodes: &str) -> String {
    format!(
        r#"{{ "data": {{ "repository": {{ "issues": {{
            "pageInfo": {{ "hasNextPage": {has_next}, "endCursor": {cursor} }},
            "nodes": {nodes}
        }} }} }} }}"#
    )
}

#[test]
fn a_page_reads_as_numbers_and_stamps_with_the_cursor_of_the_next() {
    let body = page(
        true,
        r#""abc""#,
        r#"[ { "number": 3, "updatedAt": "2026-11-09T10:00:00Z" },
             { "number": 8, "updatedAt": "2026-11-09T11:30:00Z" } ]"#,
    );

    let (index, next) = parse_issue_index_response(&body).expect("the page should parse");

    assert_eq!(
        index,
        vec![
            IssueStamp {
                number: 3,
                updated_at: "2026-11-09T10:00:00Z".to_string(),
            },
            IssueStamp {
                number: 8,
                updated_at: "2026-11-09T11:30:00Z".to_string(),
            },
        ]
    );
    assert_eq!(next.as_deref(), Some("abc"));
}

#[test]
fn the_last_page_has_no_next_cursor_and_an_empty_repo_is_an_honest_empty_index() {
    let (index, next) = parse_issue_index_response(&page(false, r#""abc""#, "[]"))
        .expect("an empty page should parse");

    assert!(index.is_empty());
    assert_eq!(next, None);
}

#[test]
fn a_response_that_cannot_be_the_whole_answer_fails_instead_of_reading_as_gone() {
    struct Case {
        name: &'static str,
        body: String,
        kind: AppErrorKind,
    }

    let cases = [
        Case {
            name: "another page promised with no cursor to reach it",
            body: page(true, "null", "[]"),
            kind: AppErrorKind::Internal,
        },
        Case {
            name: "a node GitHub could not read comes back null",
            body: page(false, "null", "[ null ]"),
            kind: AppErrorKind::Internal,
        },
        Case {
            name: "the issues connection is null beside a field error",
            body: r#"{ "data": { "repository": { "issues": null } },
                       "errors": [ { "message": "Something went wrong" } ] }"#
                .to_string(),
            kind: AppErrorKind::Internal,
        },
        Case {
            name: "a repository that did not come back",
            body: r#"{ "data": { "repository": null } }"#.to_string(),
            kind: AppErrorKind::NotFound,
        },
        Case {
            name: "the token cannot read the repository",
            body: r#"{ "data": { "repository": null },
                       "errors": [ { "type": "FORBIDDEN", "message": "No." } ] }"#
                .to_string(),
            kind: AppErrorKind::Forbidden,
        },
        Case {
            name: "rate limited, with no data at all",
            body: r#"{ "errors": [ { "message": "API rate limit exceeded" } ] }"#.to_string(),
            kind: AppErrorKind::RateLimited,
        },
    ];

    for case in cases {
        let error =
            parse_issue_index_response(&case.body).expect_err(&format!("{} must fail", case.name));
        assert_eq!(error.kind(), case.kind, "{}", case.name);
    }
}
