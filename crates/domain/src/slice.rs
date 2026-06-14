use serde::{Deserialize, Serialize};

/// The derived state of a [`Slice`] on the board.
///
/// Precedence is Blocked > WIP > Ready. `Done` (a closed Slice) is hidden from
/// the board and is therefore not represented here. The state is a pure
/// derivation over current GitHub data (computed in a later slice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SliceState {
    /// Blockers all closed, no open linked PR, and no assignee.
    Ready,
    /// An open Pull Request is linked to the Slice.
    Wip,
    /// At least one open "blocked by" dependency.
    Blocked,
}

impl SliceState {
    /// Board column order, left to right.
    pub const ALL: [SliceState; 3] = [SliceState::Ready, SliceState::Wip, SliceState::Blocked];
}

/// A read model of a GitHub issue that is a Slice of a PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// Title of the parent PRD, when known.
    pub prd_title: Option<String>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    pub state: SliceState,
}
