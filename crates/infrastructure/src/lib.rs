//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! For the walking skeleton this provides a [`FakeGitHubPort`] returning canned
//! data so the board can render end-to-end without GitHub access. The real
//! GraphQL adapter arrives in a later slice. Authentication is backed by the OS
//! secure store through [`KeyringSecureStore`] (see [`secure_store`]).

mod secure_store;

pub use secure_store::{FakeSecureStore, KeyringSecureStore};

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppResult, RawSlice, RepoRef, Slice};

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
/// Done (closed) issue that must be hidden from the board.
fn sample_raw_slices() -> Vec<RawSlice> {
    let prd = Some("Zfirot desktop dashboard".to_string());
    vec![
        // Ready: no blockers, no PR, no assignee.
        RawSlice {
            number: 4,
            title: "Live GitHub read: real board for a hardcoded repo".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
        RawSlice {
            number: 5,
            title: "Two-tier issue classification".to_string(),
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
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            open_blocker_count: 1,
        },
        RawSlice {
            number: 7,
            title: "Home screen: recent projects, reopen last".to_string(),
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
            closed: true,
            prd_title: prd,
            assignee: Some("carlos-verdes".to_string()),
            has_open_linked_pr: false,
            open_blocker_count: 0,
        },
    ]
}
