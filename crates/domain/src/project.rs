use serde::{Deserialize, Serialize};

use crate::RepoRef;

/// A read model of a GitHub repository the developer can open as a board.
///
/// Projected from GitHub each time the home screen loads (GitHub is the source
/// of truth). `pushed_at` is GitHub's ISO-8601 / RFC-3339 timestamp of the last
/// push, in UTC (`Z`); such strings sort lexically by recency, so the most
/// recently active project is simply the lexicographically greatest value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The repository this project points at.
    pub repo: RepoRef,
    /// GitHub's last-push timestamp (RFC-3339, UTC), used to order by recency.
    pub pushed_at: String,
}

impl Project {
    /// A project for `repo` last pushed at `pushed_at` (RFC-3339, UTC).
    pub fn new(repo: RepoRef, pushed_at: impl Into<String>) -> Self {
        Self {
            repo,
            pushed_at: pushed_at.into(),
        }
    }
}
