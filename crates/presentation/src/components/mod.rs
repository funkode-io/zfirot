//! Generic, reusable presentation components.
//!
//! These are callback-only: they never call the application or any API; they
//! receive data as props and emit events via callbacks, so they can be previewed
//! and tested without GitHub.

mod account_menu;
mod board_column;
mod error_banner;
mod home_screen;
mod other_issue_card;
mod prd_lane;
mod slice_card;
mod spinner;
mod token_screen;

pub use account_menu::AccountMenu;
pub use board_column::BoardColumn;
pub use error_banner::ErrorBanner;
pub use home_screen::HomeScreen;
pub use other_issue_card::OtherIssueCard;
pub use prd_lane::PrdLane;
pub use slice_card::SliceCard;
pub use spinner::{LoadingScreen, Spinner};
pub use token_screen::TokenScreen;

use domain::{BlockedReason, LinkedPrRef, PrStatus, SliceState};

/// Human-readable label for a Blocked reason — the card's chip.
pub fn blocked_reason_label(reason: BlockedReason) -> &'static str {
    match reason {
        BlockedReason::Dependency => "Dependency",
        BlockedReason::ExternalBlocker => "External blocker",
        BlockedReason::NeedsTriage => "Needs triage",
        BlockedReason::Deferred => "Deferred",
        BlockedReason::LabelledBlocked => "Labelled blocked",
    }
}

/// What the reader can do about a Blocked reason, for the chip's tooltip.
fn blocked_reason_hint(reason: BlockedReason) -> &'static str {
    match reason {
        BlockedReason::Dependency => {
            "Waiting on an open issue — frees itself when that issue closes"
        }
        BlockedReason::ExternalBlocker => {
            "Waiting on something that is not a GitHub issue — see the issue's ‘Blocked by’ section"
        }
        BlockedReason::NeedsTriage => "Labelled needs-triage — a maintainer must decide",
        BlockedReason::Deferred => "Labelled deferred — deliberately put off",
        BlockedReason::LabelledBlocked => "Labelled blocked, with no other evidence — may be stale",
    }
}

/// Tooltip for a Blocked card's chip: what the leading reason means, plus every
/// weaker reason that also applies, so showing one chip loses nothing.
pub fn blocked_reasons_tooltip(reasons: &[BlockedReason]) -> String {
    let Some((primary, rest)) = reasons.split_first() else {
        return String::new();
    };
    let mut tooltip = blocked_reason_hint(*primary).to_string();
    if !rest.is_empty() {
        let also: Vec<&str> = rest.iter().map(|r| blocked_reason_label(*r)).collect();
        tooltip.push_str("\nAlso: ");
        tooltip.push_str(&also.join(", "));
    }
    tooltip
}

/// daisyUI badge classes for a Blocked reason chip. The self-updating
/// `Dependency` reads as strongly as the state badge; the sticky, human-authored
/// reasons fade with their rank, so the weakest (`blocked` label, which can go
/// stale unnoticed) is the quietest thing on the card.
pub fn blocked_reason_badge_class(reason: BlockedReason) -> &'static str {
    match reason {
        BlockedReason::Dependency => "badge-error",
        BlockedReason::ExternalBlocker => "badge-warning",
        BlockedReason::NeedsTriage => "badge-warning",
        BlockedReason::Deferred => "badge-ghost",
        BlockedReason::LabelledBlocked => "badge-ghost",
    }
}

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

/// Human label for a PR review-lifecycle stage, shown on a Slice's PR headline.
pub fn pr_status_label(status: PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "Draft",
        PrStatus::AwaitingReview => "Awaiting review",
        PrStatus::ChangesRequested => "Changes requested",
        PrStatus::Approved => "Approved",
    }
}

/// GitHub Octicon utility class for a PR review-lifecycle stage — the exact
/// glyphs GitHub uses, for instant recognisability.
pub fn pr_status_icon_class(status: PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "icon-[octicon--git-pull-request-draft-16]",
        PrStatus::AwaitingReview => "icon-[octicon--git-pull-request-16]",
        PrStatus::ChangesRequested => "icon-[octicon--file-diff-16]",
        PrStatus::Approved => "icon-[octicon--check-circle-16]",
    }
}

/// daisyUI text color for a PR review-lifecycle stage headline.
pub fn pr_status_color(status: PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "text-base-content/50",
        PrStatus::AwaitingReview => "text-info",
        PrStatus::ChangesRequested => "text-error",
        PrStatus::Approved => "text-success",
    }
}

// A Slice's PR headline reads "Ready to merge" (Approved with no blocking
// Decorations) when the PR is ready, otherwise its review-lifecycle stage.
// "Ready to merge" is a derived reading, never a stored status (see ADR 0004).

/// Headline label for a Slice's PR.
pub fn pr_headline_label(pr: &LinkedPrRef) -> &'static str {
    if pr.is_ready_to_merge() {
        "Ready to merge"
    } else {
        pr_status_label(pr.pr_status)
    }
}

/// Headline Octicon class for a Slice's PR.
pub fn pr_headline_icon_class(pr: &LinkedPrRef) -> &'static str {
    if pr.is_ready_to_merge() {
        "icon-[octicon--git-merge-16]"
    } else {
        pr_status_icon_class(pr.pr_status)
    }
}

/// Headline color for a Slice's PR.
pub fn pr_headline_color(pr: &LinkedPrRef) -> &'static str {
    if pr.is_ready_to_merge() {
        "text-success"
    } else {
        pr_status_color(pr.pr_status)
    }
}
