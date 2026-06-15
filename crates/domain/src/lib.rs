//! Domain layer: read models, locally-owned state, and pure derivations.
//!
//! `Prd` and `Slice` are read models projected from GitHub (GitHub is the source
//! of truth). This crate has no dependencies on other layers.

mod error;
mod prose;
mod repo;
mod slice;

pub use error::{AppAction, AppError, AppErrorKind, AppResult};
pub use prose::{parse_prose, ProseLinks};
pub use repo::RepoRef;
pub use slice::{group_into_lanes, PrdLane, PrdRef, RawSlice, Slice, SliceState};
