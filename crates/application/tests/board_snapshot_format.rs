//! The board snapshot's **stored** format: what a cache written by an older
//! version of the app does when this version reads it.
//!
//! The snapshot now holds two independently-owned collections (issues and
//! Linked PRs, ADR 0008), so its serde shape changed. A snapshot in any other
//! format must fail to deserialize — that is what makes the cache adapter
//! discard it and cold-load, instead of painting a half-read board.

use application::BoardSnapshot;

#[test]
fn a_snapshot_round_trips_through_its_stored_format() {
    let snapshot = sample_snapshot();

    let json = serde_json::to_string(&snapshot).expect("a snapshot should encode");
    let restored: BoardSnapshot = serde_json::from_str(&json).expect("its own format should read");

    assert_eq!(restored, snapshot);
}

#[test]
fn a_snapshot_from_a_previous_app_version_is_rejected_rather_than_mis_read() {
    // The format that shipped before the open-PR sweep: no version marker, no
    // Linked PR collection, and PR facts nested inside each issue.
    let previous_format = r#"{
        "raw_issues": [ {
            "number": 1, "title": "A Slice", "url": "https://x/1", "body": null,
            "labels": ["slice"], "closed": false, "native_parent": null,
            "native_blockers": [], "assignee": null, "assignee_avatar_url": null,
            "linked_prs": [ {
                "number": 9, "author": "hubot", "title": "PR", "url": "https://x/pull/9",
                "pr_status": "AwaitingReview", "conflicts": false, "ci_failing": false,
                "unresolved_comment_count": 0
            } ],
            "is_native_child_of_prd": false
        } ],
        "fetched_at": "2026-07-28T07:41:22Z"
    }"#;

    serde_json::from_str::<BoardSnapshot>(previous_format)
        .expect_err("a snapshot in the previous format must not be readable");
}

#[test]
fn a_snapshot_from_a_future_format_version_is_rejected_too() {
    let mut value: serde_json::Value =
        serde_json::to_value(sample_snapshot()).expect("a snapshot should encode");
    value["version"] = serde_json::json!(u32::MAX);

    serde_json::from_value::<BoardSnapshot>(value)
        .expect_err("a snapshot from an unknown format version must not be readable");
}

/// A snapshot in the current format, produced the only way one ever is: by
/// loading a board.
fn sample_snapshot() -> BoardSnapshot {
    use application::BoardService;
    use fake_port::SweepPort;

    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime should build")
        .block_on(async {
            BoardService::new(SweepPort)
                .load(&domain::RepoRef::new("funkode-io", "zfirot"))
                .await
                .expect("the fake port should load")
                .snapshot
        })
}

mod fake_port {
    use application::GitHubPort;
    use async_trait::async_trait;
    use domain::{
        AppAction, AppResult, LinkedPrRef, PrStatus, Project, RawIssue, RawLinkedPr, RepoRef,
    };

    /// A [`GitHubPort`] returning one Slice and one open PR closing it, so a
    /// loaded snapshot exercises both stored collections.
    pub struct SweepPort;

    #[async_trait]
    impl GitHubPort for SweepPort {
        async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
            Ok(vec![RawIssue {
                number: 1,
                title: "A Slice".to_string(),
                url: "https://github.com/funkode-io/zfirot/issues/1".to_string(),
                body: None,
                labels: vec!["slice".to_string()],
                closed: false,
                native_parent: None,
                native_blockers: vec![],
                assignee: None,
                assignee_avatar_url: None,
                is_native_child_of_prd: false,
            }])
        }

        async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
            Ok(vec![RawLinkedPr {
                pr: LinkedPrRef {
                    number: 9,
                    author: Some("hubot".to_string()),
                    title: "Implement the Slice".to_string(),
                    url: "https://github.com/funkode-io/zfirot/pull/9".to_string(),
                    pr_status: PrStatus::AwaitingReview,
                    conflicts: false,
                    ci_failing: false,
                    unresolved_comment_count: 0,
                },
                closes: vec![1],
            }])
        }

        async fn list_projects(&self) -> AppResult<Vec<Project>> {
            Ok(vec![])
        }

        async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
            Ok(())
        }

        async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
            Ok(())
        }
    }
}
