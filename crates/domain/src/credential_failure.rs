use crate::error::{AppError, AppErrorKind};

/// What, if anything, is wrong with the stored credential itself, classified
/// from an `AppError` a GitHub-backed call returned. Replaces a single
/// "is this an auth failure" boolean so an Under-scoped token (valid, just
/// missing a grant) is never treated the same as an Invalid one (dead, must be
/// replaced) — see ADR 0005.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialFailure {
    /// The token itself is rejected (revoked, expired, malformed): discard it
    /// and ask for a new one.
    Invalid,
    /// The token is valid but GitHub refused it a specific grant: keep it and
    /// route to Rotate instead of losing a perfectly usable credential.
    /// `missing_permission` names the grant when GitHub's error says so; `None`
    /// when it does not (most Forbidden responses today).
    UnderScoped { missing_permission: Option<String> },
    /// Not a credential problem (network, rate limit, not found, a bug, …):
    /// the caller must not touch the token or re-route to auth.
    None,
}

impl CredentialFailure {
    /// Classify a GitHub-call failure. `Unauthorized` -> [`Invalid`](Self::Invalid),
    /// `Forbidden` -> [`UnderScoped`](Self::UnderScoped) (reading the
    /// `"missing_permission"` context key a caller may have attached),
    /// everything else -> [`None`](Self::None).
    pub fn classify(error: &AppError) -> Self {
        match error.kind() {
            AppErrorKind::Unauthorized => CredentialFailure::Invalid,
            AppErrorKind::Forbidden => CredentialFailure::UnderScoped {
                missing_permission: error.context("missing_permission").map(str::to_string),
            },
            _ => CredentialFailure::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthorized_classifies_as_invalid() {
        let error = AppError::unauthorized("GitHub rejected the token");
        assert_eq!(
            CredentialFailure::classify(&error),
            CredentialFailure::Invalid
        );
    }

    #[test]
    fn forbidden_with_a_named_permission_classifies_as_under_scoped_with_it() {
        let error = AppError::forbidden("The token lacks access to this repository")
            .with_context("missing_permission", "Issues");
        assert_eq!(
            CredentialFailure::classify(&error),
            CredentialFailure::UnderScoped {
                missing_permission: Some("Issues".to_string())
            }
        );
    }

    #[test]
    fn forbidden_without_a_named_permission_classifies_as_under_scoped_with_none() {
        let error = AppError::forbidden("The token lacks access to this repository");
        assert_eq!(
            CredentialFailure::classify(&error),
            CredentialFailure::UnderScoped {
                missing_permission: None
            }
        );
    }

    #[test]
    fn every_other_kind_classifies_as_none_and_is_not_a_credential_problem() {
        let cases = [
            AppError::internal("boom"),
            AppError::not_found("Slice does not exist"),
            AppError::rate_limited("GitHub rate limit exceeded"),
            AppError::unavailable("GitHub is temporarily unavailable"),
            AppError::invalid_input("bad input"),
            AppError::conflict("already exists"),
            AppError::business_rule_violation("nope"),
        ];
        for error in cases {
            assert_eq!(
                CredentialFailure::classify(&error),
                CredentialFailure::None,
                "expected None for {:?}",
                error.kind()
            );
        }
    }
}
