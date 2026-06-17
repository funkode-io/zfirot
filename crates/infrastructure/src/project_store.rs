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
use domain::{AppAction, AppError, AppResult, Project, RepoRef};

/// The config sub-directory and files Zfirot uses for locally-owned project
/// state: the last-opened project and the cached recent-projects list.
const CONFIG_DIR: &str = "zfirot";
const LAST_OPENED_FILE: &str = "last_opened.json";
const RECENT_PROJECTS_FILE: &str = "recent_projects.json";
const TRACKED_REPOS_FILE: &str = "tracked_repos.json";

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

    /// The cached recent-projects file, alongside the last-opened file in the
    /// same config directory.
    fn recent_projects_path(&self) -> PathBuf {
        self.path.with_file_name(RECENT_PROJECTS_FILE)
    }

    /// The tracked repos file, alongside the last-opened file in the same
    /// config directory.
    fn tracked_repos_path(&self) -> PathBuf {
        self.path.with_file_name(TRACKED_REPOS_FILE)
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

    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>> {
        let path = self.recent_projects_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // A missing or malformed file simply means "cache is cold".
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(AppError::internal("Could not read the cached projects.")
                    .with_operation("FileProjectStore::cached_projects")
                    .with_source(err))
            }
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    async fn cache_projects(&self, projects: &[Project]) -> AppAction {
        let path = self.recent_projects_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::internal("Could not create the config directory.")
                    .with_operation("FileProjectStore::cache_projects")
                    .with_source(err)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(projects).map_err(|err| {
            AppError::internal("Could not encode the cached projects.")
                .with_operation("FileProjectStore::cache_projects")
                .with_source(err)
        })?;
        std::fs::write(&path, bytes).map_err(|err| {
            AppError::internal("Could not save the cached projects.")
                .with_operation("FileProjectStore::cache_projects")
                .with_source(err)
        })
    }

    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>> {
        let path = self.tracked_repos_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // A missing or malformed file simply means "no tracked repos yet".
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(AppError::internal("Could not read the tracked repos.")
                    .with_operation("FileProjectStore::tracked_repos")
                    .with_source(err))
            }
        };
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn track_repo(&self, repo: &RepoRef) -> AppAction {
        let path = self.tracked_repos_path();
        // Read existing tracked repos
        let mut repos: Vec<RepoRef> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(AppError::internal("Could not read the tracked repos.")
                    .with_operation("FileProjectStore::track_repo")
                    .with_source(err))
            }
        };
        // Add to front if not already present (idempotent)
        if !repos.contains(repo) {
            repos.insert(0, repo.clone());
        }
        // Write back
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::internal("Could not create the config directory.")
                    .with_operation("FileProjectStore::track_repo")
                    .with_source(err)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&repos).map_err(|err| {
            AppError::internal("Could not encode the tracked repos.")
                .with_operation("FileProjectStore::track_repo")
                .with_source(err)
        })?;
        std::fs::write(&path, bytes).map_err(|err| {
            AppError::internal("Could not save the tracked repos.")
                .with_operation("FileProjectStore::track_repo")
                .with_source(err)
        })
    }

    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction {
        let path = self.tracked_repos_path();
        // Read existing tracked repos
        let mut repos: Vec<RepoRef> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(AppError::internal("Could not read the tracked repos.")
                    .with_operation("FileProjectStore::untrack_repo")
                    .with_source(err))
            }
        };
        // Remove if present
        repos.retain(|r| r != repo);
        // Write back
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::internal("Could not create the config directory.")
                    .with_operation("FileProjectStore::untrack_repo")
                    .with_source(err)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&repos).map_err(|err| {
            AppError::internal("Could not encode the tracked repos.")
                .with_operation("FileProjectStore::untrack_repo")
                .with_source(err)
        })?;
        std::fs::write(&path, bytes).map_err(|err| {
            AppError::internal("Could not save the tracked repos.")
                .with_operation("FileProjectStore::untrack_repo")
                .with_source(err)
        })
    }
}

/// An in-memory [`ProjectStorePort`] for tests.
#[derive(Debug, Default)]
pub struct FakeProjectStore {
    last_opened: Mutex<Option<RepoRef>>,
    cached_projects: Mutex<Option<Vec<Project>>>,
    tracked_repos: Mutex<Vec<RepoRef>>,
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
            cached_projects: Mutex::new(None),
            tracked_repos: Mutex::new(Vec::new()),
        }
    }

    /// A store pre-seeded with tracked repos.
    pub fn with_tracked_repos(repos: Vec<RepoRef>) -> Self {
        Self {
            last_opened: Mutex::new(None),
            cached_projects: Mutex::new(None),
            tracked_repos: Mutex::new(repos),
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

    async fn cached_projects(&self) -> AppResult<Option<Vec<Project>>> {
        Ok(self.cached_projects.lock().expect("lock poisoned").clone())
    }

    async fn cache_projects(&self, projects: &[Project]) -> AppAction {
        *self.cached_projects.lock().expect("lock poisoned") = Some(projects.to_vec());
        Ok(())
    }

    async fn tracked_repos(&self) -> AppResult<Vec<RepoRef>> {
        Ok(self.tracked_repos.lock().expect("lock poisoned").clone())
    }

    async fn track_repo(&self, repo: &RepoRef) -> AppAction {
        let mut repos = self.tracked_repos.lock().expect("lock poisoned");
        if !repos.contains(repo) {
            repos.insert(0, repo.clone());
        }
        Ok(())
    }

    async fn untrack_repo(&self, repo: &RepoRef) -> AppAction {
        let mut repos = self.tracked_repos.lock().expect("lock poisoned");
        repos.retain(|r| r != repo);
        Ok(())
    }
}
