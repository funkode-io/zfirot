//! Domain layer: read models, locally-owned state, and pure derivations.
//!
//! `Prd` and `Slice` are read models projected from GitHub (GitHub is the source
//! of truth). This crate has no dependencies on other layers.

mod error;
mod freshness;
mod prose;
mod repo;
mod slice;
mod summary;
mod token;

pub use error::{AppAction, AppError, AppErrorKind, AppResult};
pub use freshness::{format_last_updated, PollInterval};
pub use prose::{parse_prose, ProseLinks};
pub use repo::RepoRef;
pub use slice::{Blocker, RawSlice, Slice, SliceState};
pub use summary::BoardSummary;
pub use token::GitHubToken;
