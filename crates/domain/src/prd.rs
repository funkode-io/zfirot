use serde::{Deserialize, Serialize};

/// A read model of a GitHub issue that is a PRD (Product Requirements Document).
///
/// PRDs are projected from GitHub issues carrying the `prd` label; GitHub is the
/// system of record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prd {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
}
