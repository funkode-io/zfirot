//! Use-case tests for PAT authentication, run against the in-memory fake
//! [`FakeSecureStore`] — deterministic, offline, and never touching the real
//! OS keyring.

use application::AuthService;
use domain::{AppErrorKind, GitHubToken};
use infrastructure::FakeSecureStore;

const VALID_PAT: &str = "github_pat_11ABCDEFG_exampletokenvalue";

#[tokio::test]
async fn saves_a_valid_token_and_reuses_it_across_launches() {
    // First launch: no token stored, so requiring one routes to the screen.
    let store = FakeSecureStore::empty();
    let auth = AuthService::new(store);

    assert!(!auth.has_token().await.expect("query store"));

    auth.save_token(VALID_PAT).await.expect("valid PAT saves");

    assert!(auth.has_token().await.expect("query store"));
    let token = auth.require_token().await.expect("token now present");
    assert_eq!(token.expose(), VALID_PAT);
}

#[tokio::test]
async fn require_token_is_unauthorized_when_missing() {
    let auth = AuthService::new(FakeSecureStore::empty());

    let err = auth
        .require_token()
        .await
        .expect_err("no token should be unauthorized");

    assert_eq!(err.kind(), AppErrorKind::Unauthorized);
}

#[tokio::test]
async fn a_pre_seeded_token_is_available_on_launch() {
    let stored = GitHubToken::parse(VALID_PAT).expect("valid PAT");
    let auth = AuthService::new(FakeSecureStore::with_token(stored));

    let token = auth.require_token().await.expect("token persisted");
    assert_eq!(token.expose(), VALID_PAT);
}

#[tokio::test]
async fn rejects_a_non_fine_grained_token_without_storing_it() {
    let auth = AuthService::new(FakeSecureStore::empty());

    let err = auth
        .save_token("ghp_classic_token")
        .await
        .expect_err("classic tokens are rejected");

    assert_eq!(err.kind(), AppErrorKind::InvalidInput);
    assert!(
        !auth.has_token().await.expect("query store"),
        "an invalid token must not be persisted"
    );
}

#[tokio::test]
async fn clear_token_signs_out() {
    let stored = GitHubToken::parse(VALID_PAT).expect("valid PAT");
    let auth = AuthService::new(FakeSecureStore::with_token(stored));

    auth.clear_token().await.expect("clears the token");

    assert!(!auth.has_token().await.expect("query store"));
}
