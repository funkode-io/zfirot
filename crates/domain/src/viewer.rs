/// The authenticated GitHub user the stored token acts as, for the account
/// menu. Projected from GitHub's `viewer` (GitHub is the source of truth); the
/// token itself carries no profile.
///
/// See `CONTEXT.md`'s *Identity and authentication* section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    /// The GitHub handle, shown as `@login`.
    pub login: String,
    /// The display name, when the account has one set.
    pub name: Option<String>,
    /// The profile picture URL (GitHub always has one, even if a generated
    /// default).
    pub avatar_url: String,
}
