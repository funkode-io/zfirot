//! The pure join between the two authoritative collections a board is made of:
//! the open issues and the open Linked PRs swept from GitHub (ADR 0008).
//!
//! `classify` is a pure function of `(open issues, open Linked PRs)`, so every
//! case below is a table-driven, offline assertion on the rendered board — no
//! port, no cache, no clock.

use application::classify;
use domain::{LinkedPrRef, PrStatus, RawIssue, RawLinkedPr, SliceState};

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

fn swept_pr(number: u64, status: PrStatus, closes: Vec<u64>) -> RawLinkedPr {
    RawLinkedPr {
        pr: LinkedPrRef {
            number,
            author: Some("hubot".to_string()),
            title: format!("PR {number}"),
            url: format!("https://github.com/funkode-io/zfirot/pull/{number}"),
            pr_status: status,
            conflicts: false,
            ci_failing: false,
            unresolved_comment_count: 0,
        },
        closes,
    }
}

/// The `pr #n` badges a Slice renders, in board order.
fn badges(board: &application::ClassifiedBoard, issue: u64) -> Vec<u64> {
    board
        .slices
        .iter()
        .find(|slice| slice.number == issue)
        .unwrap_or_else(|| panic!("issue #{issue} should be a Slice on the board"))
        .linked_prs
        .iter()
        .map(|pr| pr.number)
        .collect()
}

fn state(board: &application::ClassifiedBoard, issue: u64) -> SliceState {
    board
        .slices
        .iter()
        .find(|slice| slice.number == issue)
        .unwrap_or_else(|| panic!("issue #{issue} should be a Slice on the board"))
        .state
}

#[test]
fn joins_swept_prs_onto_their_slices_by_closing_reference() {
    struct Case {
        name: &'static str,
        issues: Vec<u64>,
        prs: Vec<RawLinkedPr>,
        expected: Vec<(u64, Vec<u64>, SliceState)>,
    }

    let cases = [
        Case {
            name: "a Slice with no open PR stays Ready and shows no badge",
            issues: vec![1],
            prs: vec![],
            expected: vec![(1, vec![], SliceState::Ready)],
        },
        Case {
            name: "one PR closing several issues puts a badge on each of them",
            issues: vec![1, 2],
            prs: vec![swept_pr(90, PrStatus::AwaitingReview, vec![1, 2])],
            expected: vec![
                (1, vec![90], SliceState::Wip),
                (2, vec![90], SliceState::Wip),
            ],
        },
        Case {
            name: "several PRs closing one issue all show, in sweep order",
            issues: vec![1],
            prs: vec![
                swept_pr(90, PrStatus::Draft, vec![1]),
                swept_pr(91, PrStatus::Approved, vec![1]),
            ],
            expected: vec![(1, vec![90, 91], SliceState::Wip)],
        },
        Case {
            name: "a PR closing an issue the board does not hold is ignored",
            issues: vec![1],
            prs: vec![swept_pr(90, PrStatus::AwaitingReview, vec![404])],
            expected: vec![(1, vec![], SliceState::Ready)],
        },
    ];

    for case in cases {
        let issues: Vec<RawIssue> = case.issues.iter().copied().map(slice_issue).collect();

        let board = classify(&issues, &case.prs);

        for (issue, expected_badges, expected_state) in case.expected {
            assert_eq!(
                badges(&board, issue),
                expected_badges,
                "{}: unexpected PR badges on #{issue}",
                case.name
            );
            assert_eq!(
                state(&board, issue),
                expected_state,
                "{}: unexpected state on #{issue}",
                case.name
            );
        }
    }
}

#[test]
fn best_pr_is_the_highest_status_of_the_swept_prs_closing_the_slice() {
    let issues = vec![slice_issue(1)];
    let prs = vec![
        swept_pr(90, PrStatus::Draft, vec![1]),
        swept_pr(91, PrStatus::Approved, vec![1]),
        swept_pr(92, PrStatus::AwaitingReview, vec![1]),
    ];

    let board = classify(&issues, &prs);

    let slice = &board.slices[0];
    assert_eq!(
        slice.best_pr().map(|pr| (pr.number, pr.pr_status)),
        Some((91, PrStatus::Approved)),
        "the Best PR is the highest-status open PR closing the Slice"
    );
}

#[test]
fn a_pr_that_left_the_sweep_leaves_no_trace_on_its_slice() {
    // The sweep is authoritative: a merged (or closed) PR is simply absent from
    // the next sweep, and the Slice must lose its badge and its WIP state
    // without any filtering step.
    let issues = vec![slice_issue(1)];

    let with_pr = classify(&issues, &[swept_pr(90, PrStatus::Approved, vec![1])]);
    assert_eq!(state(&with_pr, 1), SliceState::Wip);

    let after_merge = classify(&issues, &[]);

    assert_eq!(badges(&after_merge, 1), Vec::<u64>::new());
    assert_eq!(
        state(&after_merge, 1),
        SliceState::Ready,
        "a PR absent from the sweep releases its Slice"
    );
}

#[test]
fn classify_is_a_pure_projection_of_issues_and_swept_prs() {
    let issues = vec![slice_issue(1), slice_issue(2)];
    let prs = vec![swept_pr(90, PrStatus::AwaitingReview, vec![1])];

    assert_eq!(
        classify(&issues, &prs),
        classify(&issues, &prs),
        "the same inputs must always classify to the same board"
    );
}

fn prd_issue(number: u64) -> RawIssue {
    RawIssue {
        labels: vec!["prd".to_string()],
        ..slice_issue(number)
    }
}

fn other_issue(number: u64) -> RawIssue {
    RawIssue {
        labels: vec![],
        ..slice_issue(number)
    }
}

#[test]
fn lists_every_open_pr_on_no_slice_card_as_unlinked() {
    struct Case {
        name: &'static str,
        issues: Vec<RawIssue>,
        prs: Vec<RawLinkedPr>,
        expected: Vec<u64>,
    }

    let cases = [
        Case {
            name: "a PR closing nothing is unlinked",
            issues: vec![slice_issue(1)],
            prs: vec![swept_pr(90, PrStatus::AwaitingReview, vec![])],
            expected: vec![90],
        },
        Case {
            name: "a PR closing a Slice is on its card, not unlinked",
            issues: vec![slice_issue(1)],
            prs: vec![swept_pr(90, PrStatus::AwaitingReview, vec![1])],
            expected: vec![],
        },
        Case {
            name: "a PR closing only a PRD is unlinked",
            issues: vec![prd_issue(1)],
            prs: vec![swept_pr(90, PrStatus::Draft, vec![1])],
            expected: vec![90],
        },
        Case {
            name: "a PR closing only an other open issue is unlinked",
            issues: vec![other_issue(1)],
            prs: vec![swept_pr(90, PrStatus::Draft, vec![1])],
            expected: vec![90],
        },
        Case {
            name: "a PR closing only an issue the board does not hold is unlinked",
            issues: vec![slice_issue(1)],
            prs: vec![swept_pr(90, PrStatus::Draft, vec![404])],
            expected: vec![90],
        },
        Case {
            name: "a PR closing a Slice and an other issue stays on the Slice alone",
            issues: vec![slice_issue(1), other_issue(2)],
            prs: vec![swept_pr(90, PrStatus::Draft, vec![1, 2])],
            expected: vec![],
        },
        Case {
            name: "unlinked PRs keep sweep order",
            issues: vec![],
            prs: vec![
                swept_pr(91, PrStatus::Draft, vec![]),
                swept_pr(90, PrStatus::Draft, vec![]),
            ],
            expected: vec![91, 90],
        },
    ];

    for case in cases {
        let board = classify(&case.issues, &case.prs);
        let unlinked: Vec<u64> = board.unlinked_prs.iter().map(|pr| pr.number).collect();
        assert_eq!(unlinked, case.expected, "{}", case.name);
    }
}

#[test]
fn two_open_prs_closing_one_open_slice_both_show_on_its_card() {
    let issues = vec![slice_issue(1)];
    let prs = vec![
        swept_pr(90, PrStatus::AwaitingReview, vec![1]),
        swept_pr(91, PrStatus::Draft, vec![1]),
    ];

    let board = classify(&issues, &prs);

    assert_eq!(badges(&board, 1), vec![90, 91]);
    assert!(
        board.unlinked_prs.is_empty(),
        "a PR on a card is not also unlinked"
    );
}

#[test]
fn a_second_pr_stays_visible_after_the_first_merges_and_closes_their_issue() {
    // Both PRs close #1. Merging #90 closes the issue and drops #90 from the
    // sweep, leaving #91 open with a closing reference to a closed issue. With
    // no card left to sit on, it must surface as an Unlinked PR.
    let swept = vec![swept_pr(91, PrStatus::AwaitingReview, vec![1])];
    let closed_issue = RawIssue {
        closed: true,
        ..slice_issue(1)
    };

    for (name, issues) in [
        ("the closed issue is absent from the board", vec![]),
        (
            "the closed issue is still held as closed",
            vec![closed_issue],
        ),
    ] {
        let board = classify(&issues, &swept);

        let unlinked: Vec<u64> = board.unlinked_prs.iter().map(|pr| pr.number).collect();
        assert_eq!(unlinked, vec![91], "{name}");
    }
}
