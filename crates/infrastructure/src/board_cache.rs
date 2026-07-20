//! Board-cache adapters implementing [`BoardCachePort`].
//!
//! [`FileBoardCache`] stores one JSON snapshot per `owner/repo` under the OS
//! config directory. [`FakeBoardCache`] keeps snapshots in memory for tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use application::{BoardCachePort, BoardSnapshot};
use async_trait::async_trait;
use domain::{AppAction, AppError, AppResult, RepoRef};

const CONFIG_DIR: &str = "zfirot";
const BOARD_CACHE_DIR: &str = "board_cache";

/// A [`BoardCachePort`] backed by JSON files in the OS config directory.
#[derive(Debug, Clone)]
pub struct FileBoardCache {
    root: PathBuf,
}

impl FileBoardCache {
    /// A cache rooted at `<config_dir>/zfirot/board_cache`.
    pub fn new() -> AppResult<Self> {
        let base = dirs::config_dir().ok_or_else(|| {
            AppError::internal("Could not locate the OS config directory.")
                .with_operation("FileBoardCache::new")
        })?;
        Ok(Self {
            root: base.join(CONFIG_DIR).join(BOARD_CACHE_DIR),
        })
    }

    /// A cache rooted at an explicit path (used in tests).
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    fn path_for(&self, repo: &RepoRef) -> PathBuf {
        self.root
            .join(&repo.owner)
            .join(format!("{}.json", repo.name))
    }
}

#[async_trait]
impl BoardCachePort for FileBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        let path = self.path_for(repo);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(AppError::internal("Could not read the cached board.")
                    .with_operation("FileBoardCache::cached_board")
                    .with_context("repo", repo)
                    .with_source(err))
            }
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
        let path = self.path_for(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::internal("Could not create the board cache directory.")
                    .with_operation("FileBoardCache::cache_board")
                    .with_context("repo", repo)
                    .with_source(err)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(snapshot).map_err(|err| {
            AppError::internal("Could not encode the cached board.")
                .with_operation("FileBoardCache::cache_board")
                .with_context("repo", repo)
                .with_source(err)
        })?;
        std::fs::write(&path, bytes).map_err(|err| {
            AppError::internal("Could not save the cached board.")
                .with_operation("FileBoardCache::cache_board")
                .with_context("repo", repo)
                .with_source(err)
        })
    }
}

/// An in-memory [`BoardCachePort`] for tests.
#[derive(Debug, Default)]
pub struct FakeBoardCache {
    snapshots: Mutex<HashMap<String, BoardSnapshot>>,
}

impl FakeBoardCache {
    /// An empty in-memory cache.
    pub fn empty() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BoardCachePort for FakeBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        Ok(self
            .snapshots
            .lock()
            .expect("lock poisoned")
            .get(&repo.to_string())
            .cloned())
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .insert(repo.to_string(), snapshot.clone());
        Ok(())
    }
}
