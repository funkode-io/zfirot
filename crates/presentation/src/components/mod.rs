//! Generic, reusable presentation components.
//!
//! These are callback-only: they never call the application or any API; they
//! receive data as props and emit events via callbacks, so they can be previewed
//! and tested without GitHub.

mod agent_action;
mod board_column;
mod error_banner;
mod home_screen;
mod other_issue_card;
mod prd_lane;
mod slice_card;
mod spinner;
mod token_screen;

pub use agent_action::{agent_action, AgentAction};
pub use board_column::BoardColumn;
pub use error_banner::ErrorBanner;
pub use home_screen::HomeScreen;
pub use other_issue_card::OtherIssueCard;
pub use prd_lane::PrdLane;
pub use slice_card::SliceCard;
pub use spinner::{LoadingScreen, Spinner};
pub use token_screen::TokenScreen;

use domain::SliceState;

/// Human-readable column/badge label for a state.
pub fn state_label(state: SliceState) -> &'static str {
    match state {
        SliceState::Ready => "Ready",
        SliceState::Wip => "WIP",
        SliceState::Blocked => "Blocked",
        SliceState::Done => "Done",
    }
}

/// daisyUI badge modifier class for a state.
pub fn state_badge_class(state: SliceState) -> &'static str {
    match state {
        SliceState::Ready => "badge-success",
        SliceState::Wip => "badge-warning",
        SliceState::Blocked => "badge-error",
        SliceState::Done => "badge-neutral",
    }
}

/// daisyUI background color class for a status dot (dot + label header).
pub fn state_dot_color(state: SliceState) -> &'static str {
    match state {
        SliceState::Ready => "bg-success",
        SliceState::Wip => "bg-warning",
        SliceState::Blocked => "bg-error",
        SliceState::Done => "bg-neutral",
    }
}
