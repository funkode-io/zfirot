//! Domain layer: read models, locally-owned state, and pure derivations.
//!
//! `Prd` and `Slice` are read models projected from GitHub (GitHub is the source
//! of truth). This crate has no dependencies on other layers.

mod classification;
mod error;
mod prd;
mod repo;
mod slice;

pub use classification::{
    classify_issue, parse_blockers_from_body, parse_parent_from_body, IssueClassification, RawIssue,
};
pub use error::{AppAction, AppError, AppErrorKind, AppResult};
pub use prd::Prd;
pub use repo::RepoRef;
pub use slice::{RawSlice, Slice, SliceState};
