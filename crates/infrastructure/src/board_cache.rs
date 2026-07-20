//! Board-cache adapters implementing [`BoardCachePort`].
//!
//! [`FileBoardCache`] stores one JSON snapshot per `owner/repo` under the OS
//! config directory. [`FakeBoardCache`] keeps snapshots in memory for tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use application::{BoardCachePort, BoardCacheUsage, BoardSnapshot, CachedProjectUsage};
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

    async fn cache_usage(&self) -> AppResult<BoardCacheUsage> {
        let mut projects = Vec::new();

        let owners = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BoardCacheUsage::default())
            }
            Err(err) => {
                return Err(AppError::internal("Could not read the board cache usage.")
                    .with_operation("FileBoardCache::cache_usage")
                    .with_source(err))
            }
        };

        for owner_entry in owners {
            let owner_entry = owner_entry.map_err(|err| {
                AppError::internal("Could not read the board cache usage.")
                    .with_operation("FileBoardCache::cache_usage")
                    .with_source(err)
            })?;
            if !owner_entry.path().is_dir() {
                continue;
            }
            let owner = match owner_entry.file_name().to_str() {
                Some(owner) => owner.to_string(),
                None => continue,
            };
            let repo_files = std::fs::read_dir(owner_entry.path()).map_err(|err| {
                AppError::internal("Could not read the board cache usage.")
                    .with_operation("FileBoardCache::cache_usage")
                    .with_source(err)
            })?;
            for file_entry in repo_files {
                let file_entry = file_entry.map_err(|err| {
                    AppError::internal("Could not read the board cache usage.")
                        .with_operation("FileBoardCache::cache_usage")
                        .with_source(err)
                })?;
                let path = file_entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let name = match path.file_stem().and_then(|stem| stem.to_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };
                let metadata = file_entry.metadata().map_err(|err| {
                    AppError::internal("Could not read the board cache usage.")
                        .with_operation("FileBoardCache::cache_usage")
                        .with_source(err)
                })?;
                projects.push(CachedProjectUsage {
                    repo: RepoRef::new(owner.clone(), name),
                    bytes: metadata.len(),
                });
            }
        }

        projects.sort_by_key(|a| a.repo.to_string());
        let total_bytes = projects.iter().map(|project| project.bytes).sum();
        Ok(BoardCacheUsage {
            projects,
            total_bytes,
        })
    }

    async fn clear_board(&self, repo: &RepoRef) -> AppAction {
        let path = self.path_for(repo);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AppError::internal("Could not clear the cached board.")
                .with_operation("FileBoardCache::clear_board")
                .with_context("repo", repo)
                .with_source(err)),
        }
    }

    async fn clear_all(&self) -> AppAction {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AppError::internal("Could not clear the board cache.")
                .with_operation("FileBoardCache::clear_all")
                .with_source(err)),
        }
    }
}

/// An in-memory [`BoardCachePort`] for tests.
#[derive(Debug, Default)]
pub struct FakeBoardCache {
    snapshots: Mutex<HashMap<String, (RepoRef, BoardSnapshot)>>,
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
            .map(|(_, snapshot)| snapshot.clone()))
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .insert(repo.to_string(), (repo.clone(), snapshot.clone()));
        Ok(())
    }

    async fn cache_usage(&self) -> AppResult<BoardCacheUsage> {
        let snapshots = self.snapshots.lock().expect("lock poisoned");
        let mut projects: Vec<CachedProjectUsage> = Vec::with_capacity(snapshots.len());
        for (repo, snapshot) in snapshots.values() {
            // Mirror the file adapter, which measures the pretty-printed JSON it
            // writes to disk; propagate encode failures instead of masking them.
            let bytes = serde_json::to_vec_pretty(snapshot).map_err(|err| {
                AppError::internal("Could not read the board cache usage.")
                    .with_operation("FakeBoardCache::cache_usage")
                    .with_context("repo", repo)
                    .with_source(err)
            })?;
            projects.push(CachedProjectUsage {
                repo: repo.clone(),
                bytes: bytes.len() as u64,
            });
        }
        drop(snapshots);
        projects.sort_by_key(|a| a.repo.to_string());
        let total_bytes = projects.iter().map(|project| project.bytes).sum();
        Ok(BoardCacheUsage {
            projects,
            total_bytes,
        })
    }

    async fn clear_board(&self, repo: &RepoRef) -> AppAction {
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .remove(&repo.to_string());
        Ok(())
    }

    async fn clear_all(&self) -> AppAction {
        self.snapshots.lock().expect("lock poisoned").clear();
        Ok(())
    }
}
