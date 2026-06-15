//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! [`GitHubClient`] is the real GraphQL adapter. [`FakeGitHubPort`] returns
//! canned data so the board can render end-to-end (and tests can run) without
//! GitHub access. Authentication is backed by the OS secure store through
//! [`KeyringSecureStore`] (see [`secure_store`]).

use application::GitHubPort;
use async_trait::async_trait;
use domain::{derive_board, AppResult, IssueRef, RawSlice, RepoRef, Slice};

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
/// so the fake exercises the same `SliceState` derivation and reverse-edge
/// resolution as the real adapter. A closed (Done) issue is included; the board
/// hides Done by rendering only [`SliceState::BOARD`], while the data is retained
/// for future use.
pub fn sample_slices() -> Vec<Slice> {
    derive_board(sample_raw_slices())
}

/// Build a blocker reference to the canned issue numbered `number`.
fn blocker_ref(number: u64) -> IssueRef {
    IssueRef {
        number,
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
    }
}

/// Raw, GitHub-shaped issues for the fake, covering every derived state plus a
/// Done (closed) issue that must be hidden from the board. The Blocked issues
/// list real open issues as blockers so the reverse "unblocks" edges resolve.
fn sample_raw_slices() -> Vec<RawSlice> {
    let prd = Some("Zfirot desktop dashboard".to_string());
    vec![
        // Ready: no blockers, no PR, no assignee. Unblocks #6 and #7.
        RawSlice {
            number: 4,
            title: "Live GitHub read: real board for a hardcoded repo".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/4".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            blockers: Vec::new(),
        },
        RawSlice {
            number: 5,
            title: "Two-tier issue classification".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/5".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            blockers: Vec::new(),
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
            blockers: Vec::new(),
        },
        // Blocked: blocked by the Ready issue #4.
        RawSlice {
            number: 6,
            title: "PAT authentication via the OS secure store".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/6".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            blockers: vec![blocker_ref(4)],
        },
        RawSlice {
            number: 7,
            title: "Home screen: recent projects, reopen last".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/7".to_string(),
            closed: false,
            prd_title: prd.clone(),
            assignee: None,
            has_open_linked_pr: false,
            blockers: vec![blocker_ref(4), blocker_ref(5)],
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
            blockers: Vec::new(),
        },
    ]
}
