//! The GraphQL payload-to-`Slice` projection, tested offline against a recorded
//! response fixture (`tests/fixtures/board.json`). This is the primary seam for
//! the real adapter: the HTTP call is exercised manually, but the mapping and
//! `SliceState` derivation are pinned here.

use domain::{RawSlice, SliceState};
use infrastructure::parse_response;

const BOARD_FIXTURE: &str = include_str!("fixtures/board.json");

fn raw_by_number(raws: &[RawSlice], number: u64) -> &RawSlice {
    raws.iter()
        .find(|raw| raw.number == number)
        .unwrap_or_else(|| panic!("no raw slice #{number} in the mapped fixture"))
}

#[test]
fn maps_native_relationships_into_raw_slices() {
    let (raws, next) = parse_response(BOARD_FIXTURE).expect("fixture should parse");

    assert_eq!(next, None, "the fixture is a single, final page");
    assert_eq!(raws.len(), 4);

    // Ready: no assignee, no open PR, no open blocker; PRD from the native parent.
    let ready = raw_by_number(&raws, 4);
    assert_eq!(ready.prd_title.as_deref(), Some("Zfirot desktop dashboard"));
    assert_eq!(ready.assignee, None);
    assert!(!ready.has_open_linked_pr);
    assert_eq!(ready.open_blocker_count, 0);
    assert!(!ready.closed);

    // WIP: an open linked PR and an assignee.
    let wip = raw_by_number(&raws, 3);
    assert_eq!(wip.assignee.as_deref(), Some("carlos-verdes"));
    assert!(wip.has_open_linked_pr);

    // Blocked: one OPEN blocker; the CLOSED one is not counted.
    let blocked = raw_by_number(&raws, 6);
    assert_eq!(blocked.open_blocker_count, 1);

    // No native parent -> no PRD tag; only-closed blockers -> not blocked.
    let orphan = raw_by_number(&raws, 8);
    assert_eq!(orphan.prd_title, None);
    assert_eq!(orphan.open_blocker_count, 0);
}

#[test]
fn derived_states_match_the_native_facts() {
    let (raws, _) = parse_response(BOARD_FIXTURE).expect("fixture should parse");

    let state_of = |number: u64| raw_by_number(&raws, number).clone().into_slice().state;

    assert_eq!(state_of(4), SliceState::Ready);
    assert_eq!(state_of(3), SliceState::Wip);
    assert_eq!(state_of(6), SliceState::Blocked);
    assert_eq!(state_of(8), SliceState::Ready);
}

#[test]
fn reports_a_query_error_as_a_failure() {
    let body = r#"{ "errors": [{ "message": "Something went wrong" }] }"#;

    let result = parse_response(body);

    assert!(
        result.is_err(),
        "GraphQL errors must surface as an AppError"
    );
}

#[test]
fn reports_a_missing_repository_as_not_found() {
    let body = r#"{ "data": { "repository": null } }"#;

    let error = parse_response(body).expect_err("a null repository must be an error");

    assert_eq!(error.kind(), domain::AppErrorKind::NotFound);
}
