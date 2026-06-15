//! Project-store adapters implementing [`ProjectStorePort`].
//!
//! [`FileProjectStore`] remembers the last-opened repository in a small JSON file
//! under the user's OS config directory, so the app can reopen that board on the
//! next launch. [`FakeProjectStore`] keeps the value in memory so use-cases can be
//! tested without touching the filesystem.

use std::path::PathBuf;
use std::sync::Mutex;

use application::ProjectStorePort;
use async_trait::async_trait;
use domain::{AppAction, AppError, AppResult, RepoRef};

/// The config sub-directory and file Zfirot uses for the last-opened project.
const CONFIG_DIR: &str = "zfirot";
const LAST_OPENED_FILE: &str = "last_opened.json";

/// A [`ProjectStorePort`] backed by a JSON file in the OS config directory.
#[derive(Debug, Clone)]
pub struct FileProjectStore {
    path: PathBuf,
}

impl FileProjectStore {
    /// A store writing to `<config_dir>/zfirot/last_opened.json`.
    pub fn new() -> AppResult<Self> {
        let base = dirs::config_dir().ok_or_else(|| {
            AppError::internal("Could not locate the OS config directory.")
                .with_operation("FileProjectStore::new")
        })?;
        Ok(Self {
            path: base.join(CONFIG_DIR).join(LAST_OPENED_FILE),
        })
    }

    /// A store writing to an explicit path (used in tests).
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }
}

#[async_trait]
impl ProjectStorePort for FileProjectStore {
    async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            // A missing or malformed file simply means "nothing remembered yet".
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(
                    AppError::internal("Could not read the last-opened project.")
                        .with_operation("FileProjectStore::last_opened")
                        .with_source(err),
                )
            }
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::internal("Could not create the config directory.")
                    .with_operation("FileProjectStore::remember_last_opened")
                    .with_source(err)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(repo).map_err(|err| {
            AppError::internal("Could not encode the last-opened project.")
                .with_operation("FileProjectStore::remember_last_opened")
                .with_source(err)
        })?;
        std::fs::write(&self.path, bytes).map_err(|err| {
            AppError::internal("Could not save the last-opened project.")
                .with_operation("FileProjectStore::remember_last_opened")
                .with_source(err)
        })
    }
}

/// An in-memory [`ProjectStorePort`] for tests.
#[derive(Debug, Default)]
pub struct FakeProjectStore {
    last_opened: Mutex<Option<RepoRef>>,
}

impl FakeProjectStore {
    /// A store with nothing remembered yet.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A store pre-seeded with a last-opened repository.
    pub fn with_last_opened(repo: RepoRef) -> Self {
        Self {
            last_opened: Mutex::new(Some(repo)),
        }
    }
}

#[async_trait]
impl ProjectStorePort for FakeProjectStore {
    async fn last_opened(&self) -> AppResult<Option<RepoRef>> {
        Ok(self.last_opened.lock().expect("lock poisoned").clone())
    }

    async fn remember_last_opened(&self, repo: &RepoRef) -> AppAction {
        *self.last_opened.lock().expect("lock poisoned") = Some(repo.clone());
        Ok(())
    }
}
