//! Infrastructure layer: adapters that implement the application's port traits.
//!
//! [`GitHubClient`] is the real GraphQL adapter. [`FakeGitHubPort`] returns
//! canned data so the board can render end-to-end (and tests can run) without
//! GitHub access. Authentication is backed by the OS secure store through
//! [`KeyringSecureStore`] (see [`secure_store`]).

use application::GitHubPort;
use async_trait::async_trait;
use domain::{AppResult, LinkedPrRef, Project, RawIssue, RepoRef};

mod board_cache;
mod github;
mod project_store;
mod secure_store;

pub use board_cache::{FakeBoardCache, FileBoardCache};
pub use github::{parse_issues_response, parse_projects_response, GitHubClient};
pub use project_store::{FakeProjectStore, FileProjectStore};
pub use secure_store::{EnvSecureStore, FakeSecureStore, KeyringSecureStore};

/// A fake [`GitHubPort`] that returns a fixed set of raw issues.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeGitHubPort;

#[async_trait]
impl GitHubPort for FakeGitHubPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(sample_raw_issues())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(sample_projects())
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> domain::AppAction {
        Ok(())
    }

    async fn add_label(
        &self,
        _repo: &RepoRef,
        _issue_number: u64,
        _label: &str,
    ) -> domain::AppAction {
        Ok(())
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

/// Canned raw issues for the fake, covering all classification tiers:
/// - Tier-1 PRD (prd label)
/// - Tier-1 Slices (ready-for-agent / slice labels, native child of PRD)
/// - Tier-2 suggested PRD (PRD template headings in body, no label)
/// - Tier-2 suggested Slice (Slice template headings in body, no label)
/// - Tier-3 unclassified (no labels, no matching headings)
/// - A closed issue (omitted by `classify_board`)
pub fn sample_raw_issues() -> Vec<RawIssue> {
    vec![
        // ── Tier-1: confirmed PRD ────────────────────────────────────────────
        RawIssue {
            number: 1,
            title: "Zfirot desktop dashboard".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/1".to_string(),
            body: Some(
                "## Problem Statement\n\nAgents need a dashboard.\n\n\
                 ## User Stories\n\n- As an agent…"
                    .to_string(),
            ),
            labels: vec!["prd".to_string()],
            closed: false,
            native_parent: None,
            native_blockers: vec![],
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            is_native_child_of_prd: false,
        },
        // ── Tier-1: confirmed Slice (ready-for-agent label) ──────────────────
        RawIssue {
            number: 3,
            title: "Derive SliceState as a pure domain function".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/3".to_string(),
            body: Some(
                "## What to build\n\nDerive state.\n\n\
                 ## Parent\n\nfunkode-io/zfirot#1\n\n\
                 ## Blocked by\n\n- #2"
                    .to_string(),
            ),
            labels: vec!["ready-for-agent".to_string()],
            closed: false,
            native_parent: Some(1),
            native_blockers: vec![2],
            assignee: Some("carlos-verdes".to_string()),
            assignee_avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            linked_prs: vec![LinkedPrRef {
                number: 12,
                author: Some("carlos-verdes".to_string()),
                title: "Derive SliceState as a pure domain function".to_string(),
                url: "https://github.com/funkode-io/zfirot/pull/12".to_string(),
                pr_status: domain::PrStatus::AwaitingReview,
                conflicts: false,
                ci_failing: false,
                unresolved_comment_count: 0,
            }],
            is_native_child_of_prd: true,
        },
        // ── Tier-1: confirmed Slice (slice label, prose parent fallback) ─────
        RawIssue {
            number: 5,
            title: "Two-tier issue classification".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/5".to_string(),
            body: Some(
                "## What to build\n\nClassify issues.\n\n\
                 ## Acceptance criteria\n\n- [ ] Labels work\n\n\
                 ## Parent\n\nfunkode-io/zfirot#1\n\n\
                 ## Blocked by\n\n- funkode-io/zfirot#3"
                    .to_string(),
            ),
            labels: vec!["slice".to_string()],
            closed: false,
            native_parent: None,
            native_blockers: vec![3, 2],
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            is_native_child_of_prd: false,
        },
        // ── Tier-2: suggested PRD (no label, but PRD headings) ───────────────
        RawIssue {
            number: 8,
            title: "Multi-repo support".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/8".to_string(),
            body: Some(
                "## Problem Statement\n\nUsers need to track multiple repos.\n\n\
                 ## User Stories\n\n- As a user, I want to add repos…"
                    .to_string(),
            ),
            labels: vec![],
            closed: false,
            native_parent: None,
            native_blockers: vec![],
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            is_native_child_of_prd: false,
        },
        // ── Tier-2: suggested Slice (no label, but Slice headings) ───────────
        RawIssue {
            number: 9,
            title: "Add dark-mode toggle".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/9".to_string(),
            body: Some(
                "## What to build\n\nA dark-mode toggle button.\n\n\
                 ## Acceptance criteria\n\n- [ ] Toggle persists across restarts"
                    .to_string(),
            ),
            labels: vec![],
            closed: false,
            native_parent: None,
            native_blockers: vec![],
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            is_native_child_of_prd: false,
        },
        // ── Tier-3: unclassified ─────────────────────────────────────────────
        RawIssue {
            number: 10,
            title: "Investigate Tauri migration".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/10".to_string(),
            body: Some("Spike: evaluate Tauri vs Dioxus desktop.".to_string()),
            labels: vec![],
            closed: false,
            native_parent: None,
            native_blockers: vec![],
            assignee: None,
            assignee_avatar_url: None,
            linked_prs: vec![],
            is_native_child_of_prd: false,
        },
        // ── Closed: omitted by classify_board ────────────────────────────────
        RawIssue {
            number: 2,
            title: "Walking skeleton".to_string(),
            url: "https://github.com/funkode-io/zfirot/issues/2".to_string(),
            body: None,
            labels: vec!["slice".to_string()],
            closed: true,
            native_parent: Some(1),
            native_blockers: vec![],
            assignee: Some("carlos-verdes".to_string()),
            assignee_avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            linked_prs: vec![],
            is_native_child_of_prd: true,
        },
    ]
}
