//! Application layer: use-cases and the port traits that infrastructure
//! implements (dependency inversion). Depends only on `domain`.

use async_trait::async_trait;
use domain::{AppResult, RepoRef, Slice};

/// The seam between the application and any GitHub backend (real or fake).
///
/// This is the primary test seam: use-cases run against a fake implementation
/// returning canned data, with no network access.
#[async_trait]
pub trait GitHubPort: Send + Sync {
    /// Load the Slices that make up a project's board.
    async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>>;
}

/// Use-cases for the project board.
pub struct BoardService<P: GitHubPort> {
    port: P,
}

impl<P: GitHubPort> BoardService<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    /// Load the board for a project.
    pub async fn load_board(&self, repo: &RepoRef) -> AppResult<Vec<Slice>> {
        self.port
            .load_board(repo)
            .await
            .map_err(|err| err.with_context("repo", repo))
    }
}
