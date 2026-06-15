//! The GraphQL payload-to-`Slice` projection, tested offline against a recorded
//! response fixture (`tests/fixtures/board.json`). This is the primary seam for
//! the real adapter: the HTTP call is exercised manually, but the mapping and
//! `SliceState` derivation are pinned here.

use domain::{derive_board, RawSlice, Slice, SliceState};
use infrastructure::{parse_response, resolve_board};

const BOARD_FIXTURE: &str = include_str!("fixtures/board.json");

/// Parse the fixture and resolve native-or-prose relationships into a board.
fn resolved_board() -> Vec<RawSlice> {
    let (issues, next) = parse_response(BOARD_FIXTURE).expect("fixture should parse");
    assert_eq!(next, None, "the fixture is a single, final page");
    resolve_board(issues)
}

fn raw_by_number(raws: &[RawSlice], number: u64) -> &RawSlice {
    raws.iter()
        .find(|raw| raw.number == number)
        .unwrap_or_else(|| panic!("no raw slice #{number} in the mapped fixture"))
}

fn slice_by_number(slices: &[Slice], number: u64) -> &Slice {
    slices
        .iter()
        .find(|slice| slice.number == number)
        .unwrap_or_else(|| panic!("no slice #{number} in the derived board"))
}

#[test]
fn maps_native_relationships_into_raw_slices() {
    let raws = resolved_board();

    assert_eq!(raws.len(), 6);

    // Ready: no assignee, no open PR, no open blocker; PRD from the native parent.
    let ready = raw_by_number(&raws, 4);
    assert_eq!(ready.prd_title.as_deref(), Some("Zfirot desktop dashboard"));
    assert_eq!(
        ready.url, "https://github.com/funkode-io/zfirot/issues/4",
        "the issue url is carried through for the clickable card"
    );
    assert_eq!(ready.assignee, None);
    assert!(!ready.has_open_linked_pr);
    assert!(ready.blockers.is_empty());
    assert!(!ready.closed);

    // WIP: an open linked PR and an assignee.
    let wip = raw_by_number(&raws, 3);
    assert_eq!(wip.assignee.as_deref(), Some("carlos-verdes"));
    assert!(wip.has_open_linked_pr);

    // Blocked: one OPEN native blocker (#4); the CLOSED one is not resolved. The
    // blocker reference carries the open issue's number + url for the badge.
    let blocked = raw_by_number(&raws, 6);
    assert_eq!(
        blocked
            .blockers
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        vec![4]
    );
    assert_eq!(
        blocked.blockers[0].url,
        "https://github.com/funkode-io/zfirot/issues/4"
    );

    // No native parent and only-closed native blockers, with no prose either.
    let orphan = raw_by_number(&raws, 8);
    assert_eq!(orphan.prd_title, None);
    assert!(orphan.blockers.is_empty());
}

#[test]
fn falls_back_to_prose_when_native_links_are_absent() {
    let raws = resolved_board();

    // #9 has no native parent or blockers; its `## Parent` / `## Blocked by`
    // prose is resolved against the fetched board.
    let prose_only = raw_by_number(&raws, 9);

    // The prose parent (#1) resolves to that issue's real title for the PRD tag.
    assert_eq!(
        prose_only.prd_title.as_deref(),
        Some("PRD: Zfirot desktop dashboard")
    );

    // Two prose blockers: #6 is open (in the fetched set) so it resolves; #99 is
    // not in the set (closed/absent) so it is omitted.
    assert_eq!(
        prose_only
            .blockers
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        vec![6]
    );
    assert_eq!(
        prose_only.clone().into_slice().state,
        SliceState::Blocked,
        "a prose blocker that is still open makes the Slice Blocked"
    );
}

#[test]
fn derives_reverse_unblocks_edges_across_the_board() {
    let board = derive_board(resolved_board());

    // #6 is blocked by #4 (native), and #9 is blocked by #6 (prose). So #4
    // unblocks #6, and #6 unblocks #9. The reverse edges carry each blocked
    // issue's number + url for clickable badges.
    let four = slice_by_number(&board, 4);
    assert_eq!(
        four.unblocks.iter().map(|r| r.number).collect::<Vec<_>>(),
        vec![6],
        "#4 unblocks #6"
    );
    assert_eq!(
        four.unblocks[0].url,
        "https://github.com/funkode-io/zfirot/issues/6"
    );

    let six = slice_by_number(&board, 6);
    assert_eq!(
        six.unblocks.iter().map(|r| r.number).collect::<Vec<_>>(),
        vec![9],
        "#6 unblocks #9"
    );

    // Closed/absent blockers never produce a reverse edge: no slice unblocks the
    // off-board issues (#10, #11, #99) referenced only by closed/missing links.
    for ghost in [10, 11, 99] {
        assert!(
            board.iter().all(|s| s.number != ghost),
            "#{ghost} is off-board and must not appear"
        );
        assert!(
            board
                .iter()
                .all(|s| s.unblocks.iter().all(|r| r.number != ghost)),
            "no slice should claim to unblock off-board #{ghost}"
        );
    }
}

#[test]
fn derived_states_match_the_native_facts() {
    let raws = resolved_board();

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

#[test]
fn reports_a_repository_not_found_graphql_error_as_not_found() {
    let body = r#"{ "errors": [{ "type": "NOT_FOUND", "message": "Could not resolve to a Repository with the name 'funkode-io/missing'." }] }"#;

    let error = parse_response(body).expect_err("a NOT_FOUND GraphQL error must map to NotFound");

    assert_eq!(error.kind(), domain::AppErrorKind::NotFound);
}
