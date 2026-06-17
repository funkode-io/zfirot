use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A reference to a GitHub repository the user is viewing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// Parse an `"owner/repo"` string: trim surrounding whitespace, require
    /// exactly one `/`, and validate that both segments are non-empty and
    /// contain only GitHub-valid characters.
    ///
    /// Owner valid characters: ASCII letters, digits, and hyphens; must not
    /// start or end with a hyphen.
    ///
    /// Repository name valid characters: ASCII letters, digits, hyphens,
    /// underscores, and dots.
    ///
    /// Returns [`AppErrorKind::InvalidInput`] for any violation.
    pub fn parse(raw: impl Into<String>) -> AppResult<Self> {
        let trimmed = raw.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(AppError::invalid_input("Enter a repository as owner/repo.")
                .with_operation("RepoRef::parse"));
        }

        let slash_count = trimmed.chars().filter(|&c| c == '/').count();
        if slash_count == 0 {
            return Err(AppError::invalid_input(
                "Enter a repository as owner/repo (e.g. \"octocat/hello-world\").",
            )
            .with_operation("RepoRef::parse"));
        }
        if slash_count > 1 {
            return Err(AppError::invalid_input(
                "Enter a repository as owner/repo — only one slash is allowed.",
            )
            .with_operation("RepoRef::parse"));
        }

        // SAFETY: exactly one slash was confirmed above
        let (owner, name) = trimmed.split_once('/').unwrap();

        if owner.is_empty() {
            return Err(AppError::invalid_input("Owner must not be empty.")
                .with_operation("RepoRef::parse"));
        }
        if name.is_empty() {
            return Err(
                AppError::invalid_input("Repository name must not be empty.")
                    .with_operation("RepoRef::parse"),
            );
        }

        // GitHub owner: ASCII letters, digits, and hyphens; no leading/trailing hyphen.
        if owner.starts_with('-')
            || owner.ends_with('-')
            || !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(AppError::invalid_input(
                "Owner must contain only letters, digits, and hyphens, \
                 and may not start or end with a hyphen.",
            )
            .with_operation("RepoRef::parse"));
        }

        // GitHub repo name: ASCII letters, digits, hyphens, underscores, and dots.
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(AppError::invalid_input(
                "Repository name must contain only letters, digits, hyphens, underscores, and dots.",
            )
            .with_operation("RepoRef::parse"));
        }

        Ok(Self::new(owner, name))
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppErrorKind;

    struct Case {
        input: &'static str,
        ok: Option<(&'static str, &'static str)>,
    }

    #[test]
    fn parse_table() {
        let cases: &[Case] = &[
            // Valid
            Case {
                input: "octocat/hello-world",
                ok: Some(("octocat", "hello-world")),
            },
            Case {
                input: "  funkode-io/zfirot  ",
                ok: Some(("funkode-io", "zfirot")),
            },
            Case {
                input: "owner/repo_name.ext",
                ok: Some(("owner", "repo_name.ext")),
            },
            // Empty input
            Case {
                input: "",
                ok: None,
            },
            Case {
                input: "   ",
                ok: None,
            },
            // No slash
            Case {
                input: "ownerrepo",
                ok: None,
            },
            // Multi-slash
            Case {
                input: "owner/repo/extra",
                ok: None,
            },
            // Empty owner
            Case {
                input: "/repo",
                ok: None,
            },
            // Empty name
            Case {
                input: "owner/",
                ok: None,
            },
            // Invalid owner characters
            Case {
                input: "owner name/repo",
                ok: None,
            },
            Case {
                input: "-owner/repo",
                ok: None,
            },
            Case {
                input: "owner-/repo",
                ok: None,
            },
            // Invalid repo name characters
            Case {
                input: "owner/repo name",
                ok: None,
            },
            Case {
                input: "owner/repo@1",
                ok: None,
            },
        ];

        for case in cases {
            match (RepoRef::parse(case.input), case.ok) {
                (Ok(repo), Some((owner, name))) => {
                    assert_eq!(repo.owner, owner, "input={:?}", case.input);
                    assert_eq!(repo.name, name, "input={:?}", case.input);
                }
                (Ok(repo), None) => {
                    panic!("expected parse({:?}) to fail, but got {repo:?}", case.input);
                }
                (Err(err), None) => {
                    assert_eq!(
                        err.kind(),
                        AppErrorKind::InvalidInput,
                        "input={:?}",
                        case.input
                    );
                }
                (Err(err), Some(_)) => {
                    panic!(
                        "expected parse({:?}) to succeed, but got {err:?}",
                        case.input
                    );
                }
            }
        }
    }
}
