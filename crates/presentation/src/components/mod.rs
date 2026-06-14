//! Generic, reusable presentation components.
//!
//! These are callback-only: they never call the application or any API; they
//! receive data as props and emit events via callbacks, so they can be previewed
//! and tested without GitHub.

mod board_column;
mod slice_card;

pub use board_column::BoardColumn;
pub use slice_card::SliceCard;

use domain::SliceState;

/// Human-readable column/badge label for a state.
pub fn state_label(state: SliceState) -> &'static str {
    match state {
        SliceState::Ready => "Ready",
        SliceState::Wip => "WIP",
        SliceState::Blocked => "Blocked",
    }
}

/// daisyUI badge modifier class for a state.
pub fn state_badge_class(state: SliceState) -> &'static str {
    match state {
        SliceState::Ready => "badge-success",
        SliceState::Wip => "badge-warning",
        SliceState::Blocked => "badge-error",
    }
}
