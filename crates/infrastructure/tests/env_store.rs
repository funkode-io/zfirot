//! Tests for the development-only [`EnvSecureStore`], which reads the token from
//! an environment variable so repeated `dx serve` rebuilds do not re-trigger the
//! OS keychain prompt. Each test uses a unique variable name to stay independent.

use application::SecureStorePort;
use infrastructure::EnvSecureStore;

const VALID_PAT: &str = "github_pat_11ABCDEFG_exampletokenvalue";

#[tokio::test]
async fn reads_a_valid_token_from_the_environment() {
    let var = "ZFIROT_ENV_STORE_PRESENT";
    std::env::set_var(var, VALID_PAT);

    let store = EnvSecureStore::new(var);
    let token = store
        .load_token()
        .await
        .expect("env read")
        .expect("token present");

    assert_eq!(token.expose(), VALID_PAT);

    std::env::remove_var(var);
}

#[tokio::test]
async fn an_unset_variable_reads_as_no_token() {
    let store = EnvSecureStore::new("ZFIROT_ENV_STORE_UNSET");

    assert!(store.load_token().await.expect("env read").is_none());
}

#[tokio::test]
async fn a_malformed_value_reads_as_no_token() {
    let var = "ZFIROT_ENV_STORE_MALFORMED";
    std::env::set_var(var, "not-a-fine-grained-token");

    let store = EnvSecureStore::new(var);
    assert!(store.load_token().await.expect("env read").is_none());

    std::env::remove_var(var);
}

#[tokio::test]
async fn saving_and_deleting_are_no_ops() {
    let token = domain::GitHubToken::parse(VALID_PAT).expect("valid PAT");
    let store = EnvSecureStore::new("ZFIROT_ENV_STORE_NOOP");

    store.save_token(&token).await.expect("save is a no-op");
    store.delete_token().await.expect("delete is a no-op");
}
