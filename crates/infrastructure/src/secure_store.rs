//! Secure-store adapters implementing [`SecureStorePort`].
//!
//! [`KeyringSecureStore`] persists the Personal Access Token in the operating
//! system's credential store via the `keyring` crate (Keychain on macOS,
//! Credential Manager on Windows, kernel keyutils on Linux). [`FakeSecureStore`]
//! keeps the token in memory so use-cases can be tested without touching the
//! real keyring.

use std::sync::Mutex;

use application::SecureStorePort;
use async_trait::async_trait;
use domain::{AppAction, AppError, AppResult, GitHubToken};

/// The credential's service/account coordinates in the OS secure store.
const KEYRING_SERVICE: &str = "io.funkode.zfirot";
const KEYRING_ACCOUNT: &str = "github-pat";

/// A [`SecureStorePort`] backed by the OS secure store (keyring).
#[derive(Debug, Clone)]
pub struct KeyringSecureStore {
    service: String,
    account: String,
}

impl Default for KeyringSecureStore {
    fn default() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: KEYRING_ACCOUNT.to_string(),
        }
    }
}

impl KeyringSecureStore {
    /// A store using the default Zfirot service/account coordinates.
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&self) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account).map_err(|err| {
            AppError::internal("Could not access the OS secure store.")
                .with_operation("KeyringSecureStore::entry")
                .with_source(err)
        })
    }
}

#[async_trait]
impl SecureStorePort for KeyringSecureStore {
    async fn save_token(&self, token: &GitHubToken) -> AppAction {
        self.entry()?.set_password(token.expose()).map_err(|err| {
            AppError::internal("Could not save the Personal Access Token.")
                .with_operation("KeyringSecureStore::save_token")
                .with_source(err)
        })
    }

    async fn load_token(&self) -> AppResult<Option<GitHubToken>> {
        match self.entry()?.get_password() {
            Ok(secret) => Ok(Some(GitHubToken::from_stored(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(
                AppError::internal("Could not read the Personal Access Token.")
                    .with_operation("KeyringSecureStore::load_token")
                    .with_source(err),
            ),
        }
    }

    async fn delete_token(&self) -> AppAction {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(
                AppError::internal("Could not remove the Personal Access Token.")
                    .with_operation("KeyringSecureStore::delete_token")
                    .with_source(err),
            ),
        }
    }
}

/// An in-memory [`SecureStorePort`] for tests: deterministic, offline, and
/// never touching the real OS keyring.
#[derive(Debug, Default)]
pub struct FakeSecureStore {
    token: Mutex<Option<GitHubToken>>,
}

impl FakeSecureStore {
    /// An empty store, as if no token has been saved yet.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A store pre-seeded with a token, as if one were saved on a prior launch.
    pub fn with_token(token: GitHubToken) -> Self {
        Self {
            token: Mutex::new(Some(token)),
        }
    }
}

#[async_trait]
impl SecureStorePort for FakeSecureStore {
    async fn save_token(&self, token: &GitHubToken) -> AppAction {
        *self.token.lock().expect("secure-store mutex poisoned") = Some(token.clone());
        Ok(())
    }

    async fn load_token(&self) -> AppResult<Option<GitHubToken>> {
        Ok(self
            .token
            .lock()
            .expect("secure-store mutex poisoned")
            .clone())
    }

    async fn delete_token(&self) -> AppAction {
        *self.token.lock().expect("secure-store mutex poisoned") = None;
        Ok(())
    }
}
