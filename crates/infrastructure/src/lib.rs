//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! [`GitHubClient`] is the real GraphQL adapter. [`FakeGitHubPort`] returns
//! canned data so the board can render end-to-end (and tests can run) without
//! GitHub access. Authentication is backed by the OS secure store through
//! [`KeyringSecureStore`] (see [`secure_store`]).

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppResult, Project, RawSlice, RepoRef, Slice};

mod github;
mod project_store;
mod secure_store;

pub use github::{parse_projects_response, parse_response, resolve_board, GitHubClient, RawIssue};
pub use project_store::{FakeProjectStore, FileProjectStore};
pub use secure_store::{EnvSecureStore, FakeSecureStore, KeyringSecureStore};

/// A fake [`GitHubPort`] that returns a fixed set of Slices.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeGitHubPort;

#[async_trait]
impl GitHubPort for FakeGitHubPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(sample_slices())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(sample_projects())
    }
}

/// Canned recent projects for the fake, with varied `pushed_at` timestamps so
/// callers can exercise the most-recently-pushed-first ordering.
pub fn sample_projects() -> Vec<Project> {
    vec![
        Project::new(
            RepoRef::new("funkode-io", "zfirot"),
            "2024-05-01T12:00:00Z".to_string(),
        ),
        Project::new(
            RepoRef::new("funkode-io", "replay"),
            "2024-04-15T09:30:00Z".to_string(),
        ),
        Project::new(
            RepoRef::new("carlos-verdes", "dotfiles"),
            "2024-03-20T18:45:00Z".to_string(),
        ),
    ]
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
/// Done (closed) issue that must be hidden from the board.
fn sample_raw_slices() -> Vec<RawSlice> {
    let prd = Some("Zfirot desktop dashboard".to_string());
    vec![
        // Ready: no blockers, no PR, no assignee.
        RawSlice {
            number: 4,
            title: "Live GitHub read: real board for a hardcoded repo".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/4".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        RawSlice {
            number: 5,
            title: "Two-tier issue classification".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/5".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        // WIP: an open Pull Request is linked.
        RawSlice {
            number: 3,
            title: "Derive SliceState as a pure domain function".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/3".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: Some("carlos-verdes".to_string()),
            has_open_linked_pr: true,
            open_blocker_count: 0,
        },
        // Blocked: at least one open "blocked by" dependency.
        RawSlice {
            number: 6,
            title: "PAT authentication via the OS secure store".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/6".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 1,
        },
        RawSlice {
            number: 7,
            title: "Home screen: recent projects, reopen last".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/7".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 2,
        },
        // Done: closed, so hidden from the board.
        RawSlice {
            number: 2,
            title: "Walking skeleton".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/2".to_string(),
            closed: true,
            prd_title: prd,
            assignee: Some("carlos-verdes".to_string()),
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
    ]
}
