//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! [`GitHubClient`] is the real GraphQL adapter. [`FakeGitHubPort`] returns
//! canned data so the board can render end-to-end (and tests can run) without
//! GitHub access. Authentication is backed by the OS secure store through
//! [`KeyringSecureStore`] (see [`secure_store`]).

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppResult, PrdRef, RawSlice, RepoRef, Slice};

mod github;
mod secure_store;

pub use github::{parse_response, resolve_board, GitHubClient, RawIssue};
pub use secure_store::{EnvSecureStore, FakeSecureStore, KeyringSecureStore};

/// A fake [`GitHubPort`] that returns a fixed set of Slices.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeGitHubPort;

#[async_trait]
impl GitHubPort for FakeGitHubPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(sample_slices())
    }
}

/// Canned board Slices spanning every state, derived from raw GitHub-shaped data
/// so the fake exercises the same `SliceState` derivation as the real adapter
/// will. A closed (Done) issue is included; the board hides Done by rendering
/// only [`SliceState::BOARD`], while the data is retained for future use.
pub fn sample_slices() -> Vec<Slice> {
    sample_raw_slices()
        .into_iter()
        .map(RawSlice::into_slice)
        .collect()
}

/// Raw, GitHub-shaped issues for the fake, covering every derived state plus a
/// Done (closed) issue that must be hidden from the board. The Slices span two
/// PRDs and one issue with no PRD, so the fake exercises the board's swimlane
/// grouping (one lane per PRD plus a "No PRD" lane).
fn sample_raw_slices() -> Vec<RawSlice> {
    let dashboard = PrdRef {
        number: 1,
        title: "Zfirot desktop dashboard".to_string(),
        url: "https://github.com/funkode-io/zfirot/issues/1".to_string(),
    };
    let classification = PrdRef {
        number: 10,
        title: "Issue classification & tagging".to_string(),
        url: "https://github.com/funkode-io/zfirot/issues/10".to_string(),
    };
    vec![
        // Dashboard PRD, Ready: no blockers, no PR, no assignee.
        RawSlice {
            number: 4,
            title: "Live GitHub read: real board for a hardcoded repo".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/4".to_string(),
            closed: false,
            prd: Some(dashboard.clone()),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        // Dashboard PRD, WIP: an open Pull Request is linked.
        RawSlice {
            number: 3,
            title: "Derive SliceState as a pure domain function".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/3".to_string(),
            closed: false,
            prd: Some(dashboard.clone()),
            assignee: Some("carlos-verdes".to_string()),
            has_open_linked_pr: true,
            open_blocker_count: 0,
        },
        // Dashboard PRD, Blocked: at least one open "blocked by" dependency.
        RawSlice {
            number: 7,
            title: "Home screen: recent projects, reopen last".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/7".to_string(),
            closed: false,
            prd: Some(dashboard.clone()),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 2,
        },
        // Classification PRD, Ready.
        RawSlice {
            number: 5,
            title: "Two-tier issue classification".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/5".to_string(),
            closed: false,
            prd: Some(classification.clone()),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        // Classification PRD, Blocked.
        RawSlice {
            number: 6,
            title: "PAT authentication via the OS secure store".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/6".to_string(),
            closed: false,
            prd: Some(classification),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 1,
        },
        // No PRD: lands in the trailing "No PRD" lane.
        RawSlice {
            number: 11,
            title: "Investigate flaky GraphQL pagination".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/11".to_string(),
            closed: false,
            prd: None,
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        // Done: closed, so hidden from the board.
        RawSlice {
            number: 2,
            title: "Walking skeleton".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/2".to_string(),
            closed: true,
            prd: Some(dashboard),
            assignee: Some("carlos-verdes".to_string()),
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
    ]
}
