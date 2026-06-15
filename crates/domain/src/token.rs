use std::fmt;

use crate::error::{AppError, AppResult};

/// A GitHub fine-grained Personal Access Token, validated for shape only.
///
/// The secret is kept private and never rendered: `Debug` redacts it and there
/// is no `Display`, so a token can never leak into logs or an [`AppError`]
/// chain. Whether the token is actually accepted by GitHub (valid, with
/// sufficient scopes) is decided by the GraphQL adapter, not here.
#[derive(Clone, PartialEq, Eq)]
pub struct GitHubToken(String);

impl GitHubToken {
    /// The prefix GitHub issues fine-grained Personal Access Tokens with.
    const FINE_GRAINED_PREFIX: &'static str = "github_pat_";

    /// Parse a pasted token: trim surrounding whitespace and check it has the
    /// fine-grained PAT shape. The secret is never echoed back in errors.
    pub fn parse(raw: impl Into<String>) -> AppResult<Self> {
        let trimmed = raw.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(AppError::invalid_input("Enter a Personal Access Token.")
                .with_operation("GitHubToken::parse"));
        }
        if !trimmed.starts_with(Self::FINE_GRAINED_PREFIX) {
            return Err(AppError::invalid_input(
                "Enter a fine-grained Personal Access Token (it starts with \"github_pat_\").",
            )
            .with_operation("GitHubToken::parse"));
        }
        Ok(Self(trimmed))
    }

    /// Wrap an already-validated secret read back from the secure store.
    pub fn from_stored(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The raw secret, for authorising requests. Treat the result as sensitive.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GitHubToken {
    /// Redacted so the secret never reaches logs or an error chain.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GitHubToken(***redacted***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppErrorKind;

    #[test]
    fn parses_a_fine_grained_token() {
        let token = GitHubToken::parse("github_pat_ABC123").expect("valid PAT");
        assert_eq!(token.expose(), "github_pat_ABC123");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let token = GitHubToken::parse("  github_pat_ABC123\n").expect("valid PAT");
        assert_eq!(token.expose(), "github_pat_ABC123");
    }

    #[test]
    fn rejects_an_empty_token() {
        let err = GitHubToken::parse("   ").expect_err("empty is invalid");
        assert_eq!(err.kind(), AppErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_a_classic_or_malformed_token() {
        let err = GitHubToken::parse("ghp_classic_token").expect_err("not fine-grained");
        assert_eq!(err.kind(), AppErrorKind::InvalidInput);
    }

    #[test]
    fn debug_redacts_the_secret() {
        let token = GitHubToken::parse("github_pat_SECRET").expect("valid PAT");
        let rendered = format!("{token:?}");
        assert!(!rendered.contains("SECRET"), "secret must not be rendered");
        assert!(rendered.contains("redacted"));
    }
}
