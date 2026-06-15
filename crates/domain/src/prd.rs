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

/// The identity of the PRD a [`crate::Slice`] belongs to.
///
/// Unlike the bare PRD title, this carries the issue **number** (so board lanes
/// are keyed stably) and the **url** (so a lane header can link to the PRD issue
/// on GitHub). Resolved from a Slice's native parent or its prose `## Parent`
/// reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrdRef {
    /// The PRD issue number.
    pub number: u64,
    pub title: String,
    /// The PRD issue's URL on GitHub.
    pub url: String,
}
