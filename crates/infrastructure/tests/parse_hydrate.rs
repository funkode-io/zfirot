//! The GraphQL payload-to-[`RawIssue`] projection of a **Hydrate**, tested offline
//! against recorded response bodies. What matters here is how an unresolvable
//! number reads, since a wrong call either fails a whole refresh or drops a live
//! Slice off the board. The HTTP call itself is exercised manually.

use domain::AppErrorKind;
use infrastructure::parse_hydrate_response;

fn issue_node(number: u64, state: &str) -> String {
    format!(
        r#"{{
            "number": {number}, "title": "Slice {number}",
            "url": "https://github.com/funkode-io/zfirot/issues/{number}",
            "body": "", "state": "{state}",
            "labels": {{ "nodes": [ {{ "name": "slice" }} ] }},
            "assignees": {{ "nodes": [ {{ "login": "colleague", "avatarUrl": "https://a/1" }} ] }},
            "parent": null,
            "blockedBy": {{ "nodes": [ {{ "number": 3 }} ] }}
        }}"#
    )
}

#[test]
fn maps_each_alias_to_an_issue_sorted_by_number() {
    let body = format!(
        r#"{{ "data": {{ "repository": {{ "i7": {}, "i12": {} }} }} }}"#,
        issue_node(12, "CLOSED"),
        issue_node(7, "OPEN"),
    );
    // The aliases deliberately disagree with the nodes: the node's number is the
    // fact, and the alias carries nothing.
    let issues = parse_hydrate_response(&body).expect("the body should parse");

    let numbers: Vec<u64> = issues.iter().map(|issue| issue.number).collect();
    assert_eq!(numbers, vec![7, 12]);
    assert!(!issues[0].closed);
    assert!(issues[1].closed, "a closed issue reads closed, not absent");
    assert_eq!(issues[0].assignee.as_deref(), Some("colleague"));
    assert_eq!(issues[0].native_blockers, vec![3]);
    assert_eq!(issues[0].labels, vec!["slice".to_string()]);
}

#[test]
fn an_unresolvable_number_is_left_out_and_the_rest_of_the_batch_stands() {
    let body = format!(
        r#"{{
            "data": {{ "repository": {{ "i7": {}, "i9": null }} }},
            "errors": [ {{
                "type": "NOT_FOUND",
                "path": ["repository", "i9"],
                "message": "Could not resolve to an Issue with the number of 9."
            }} ]
        }}"#,
        issue_node(7, "OPEN"),
    );

    let issues = parse_hydrate_response(&body).expect("a missing issue is not a failure");

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 7);
}

#[test]
fn a_batch_of_only_unresolvable_numbers_is_an_empty_answer() {
    let body = r#"{
        "data": { "repository": { "i9": null } },
        "errors": [ { "type": "NOT_FOUND", "message": "Could not resolve to an Issue with the number of 9." } ]
    }"#;

    assert!(parse_hydrate_response(body)
        .expect("a missing issue is not a failure")
        .is_empty());
}

#[test]
fn a_null_alias_for_any_other_reason_fails_rather_than_dropping_a_live_slice() {
    let body = format!(
        r#"{{
            "data": {{ "repository": {{ "i7": {}, "i9": null, "i10": null }} }},
            "errors": [
                {{ "type": "NOT_FOUND", "message": "Could not resolve to an Issue with the number of 9." }},
                {{ "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" }}
            ]
        }}"#,
        issue_node(7, "OPEN"),
    );

    let error = parse_hydrate_response(&body).expect_err("a FORBIDDEN alias may still exist");

    assert_eq!(error.kind(), AppErrorKind::Forbidden);
}

#[test]
fn a_field_error_beside_a_resolved_issue_is_tolerated() {
    let body = format!(
        r#"{{
            "data": {{ "repository": {{ "i7": {} }} }},
            "errors": [ {{ "type": "FORBIDDEN", "message": "Resource not accessible by personal access token" }} ]
        }}"#,
        issue_node(7, "OPEN"),
    );

    let issues = parse_hydrate_response(&body).expect("partial field errors must not fail");

    assert_eq!(issues.len(), 1);
}

#[test]
fn an_unreadable_repository_is_a_failure_not_an_empty_answer() {
    let not_found = r#"{
        "data": { "repository": null },
        "errors": [ { "type": "NOT_FOUND", "message": "Could not resolve to a Repository with the name 'a/b'." } ]
    }"#;
    assert_eq!(
        parse_hydrate_response(not_found)
            .expect_err("no repository")
            .kind(),
        AppErrorKind::NotFound,
    );

    assert_eq!(
        parse_hydrate_response(r#"{ "data": { "repository": null } }"#)
            .expect_err("no repository")
            .kind(),
        AppErrorKind::NotFound,
    );

    assert_eq!(
        parse_hydrate_response("not json")
            .expect_err("malformed")
            .kind(),
        AppErrorKind::Internal,
    );
}
