//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! For the walking skeleton this provides a [`FakeGitHubPort`] returning canned
//! data so the board can render end-to-end without GitHub access. The real
//! GraphQL adapter arrives in a later slice.

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppResult, RepoRef, Slice, SliceState};

/// A fake [`GitHubPort`] that returns a fixed set of Slices.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeGitHubPort;

#[async_trait]
impl GitHubPort for FakeGitHubPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(sample_slices())
    }
}

/// Canned Slices spanning every board state, for previews and tests.
pub fn sample_slices() -> Vec<Slice> {
    let prd = Some("Zfirot desktop dashboard".to_string());
    vec![
        Slice {
            number: 4,
            title: "Live GitHub read: real board for a hardcoded repo".to_string(),
            prd_title: prd.clone(),
            assignee: None,
            state: SliceState::Ready,
        },
        Slice {
            number: 5,
            title: "Two-tier issue classification".to_string(),
            prd_title: prd.clone(),
            assignee: None,
            state: SliceState::Ready,
        },
        Slice {
            number: 3,
            title: "Derive SliceState as a pure domain function".to_string(),
            prd_title: prd.clone(),
            assignee: Some("carlos-verdes".to_string()),
            state: SliceState::Wip,
        },
        Slice {
            number: 6,
            title: "PAT authentication via the OS secure store".to_string(),
            prd_title: prd.clone(),
            assignee: None,
            state: SliceState::Blocked,
        },
        Slice {
            number: 7,
            title: "Home screen: recent projects, reopen last".to_string(),
            prd_title: prd,
            assignee: None,
            state: SliceState::Blocked,
        },
    ]
}
