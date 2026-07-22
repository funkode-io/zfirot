use serde::{Deserialize, Serialize};

/// GitHub's review decision for a Pull Request — the review-lifecycle input to
/// [`PrStatus`]. Mirrors GitHub's `PullRequestReviewDecision`, minus the states
/// the board does not distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    /// At least one approval and no outstanding change requests.
    Approved,
    /// A reviewer requested changes; the author must act.
    ChangesRequested,
    /// Review is required but no decision has been reached yet.
    ReviewRequired,
}

/// The review-lifecycle stage of an open Linked PR — a single ordered axis that
/// answers "whose court is the ball in?". Merge-health signals (conflicts, CI,
/// unresolved comments) are separate Decorations that ride on top; they are not
/// stages of this axis. See ADR 0004 and the `CONTEXT.md` "Pull request status"
/// glossary.
///
/// The variants are declared — and therefore `Ord`-ordered — from furthest from
/// done to done: `Draft < AwaitingReview < ChangesRequested < Approved`. That
/// ordering lets a Slice pick its **Best PR** (the maximum) as its headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrStatus {
    /// Author still working; not up for review.
    Draft,
    /// Author is done; waiting on a reviewer.
    AwaitingReview,
    /// Reviewer bounced it back; author must act.
    ChangesRequested,
    /// Approved; waiting on the merger.
    Approved,
}

impl PrStatus {
    /// Derive the review-lifecycle stage from GitHub facts.
    ///
    /// A draft PR is always [`PrStatus::Draft`], regardless of any review
    /// decision. Otherwise the review decision maps directly, and a PR that is
    /// merely awaiting a first review (`ReviewRequired`, or no decision at all)
    /// is [`PrStatus::AwaitingReview`]. Pure and total, so it is table-tested
    /// offline.
    pub fn derive(is_draft: bool, review_decision: Option<ReviewDecision>) -> Self {
        if is_draft {
            return PrStatus::Draft;
        }
        match review_decision {
            Some(ReviewDecision::Approved) => PrStatus::Approved,
            Some(ReviewDecision::ChangesRequested) => PrStatus::ChangesRequested,
            Some(ReviewDecision::ReviewRequired) | None => PrStatus::AwaitingReview,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_review_lifecycle_stage_from_github_facts() {
        struct Case {
            name: &'static str,
            is_draft: bool,
            review_decision: Option<ReviewDecision>,
            expected: PrStatus,
        }
        let cases = [
            Case {
                name: "draft wins over any review decision",
                is_draft: true,
                review_decision: Some(ReviewDecision::Approved),
                expected: PrStatus::Draft,
            },
            Case {
                name: "draft with no decision -> Draft",
                is_draft: true,
                review_decision: None,
                expected: PrStatus::Draft,
            },
            Case {
                name: "approved -> Approved",
                is_draft: false,
                review_decision: Some(ReviewDecision::Approved),
                expected: PrStatus::Approved,
            },
            Case {
                name: "changes requested -> ChangesRequested",
                is_draft: false,
                review_decision: Some(ReviewDecision::ChangesRequested),
                expected: PrStatus::ChangesRequested,
            },
            Case {
                name: "review required -> AwaitingReview",
                is_draft: false,
                review_decision: Some(ReviewDecision::ReviewRequired),
                expected: PrStatus::AwaitingReview,
            },
            Case {
                name: "no decision (review not required) -> AwaitingReview",
                is_draft: false,
                review_decision: None,
                expected: PrStatus::AwaitingReview,
            },
        ];
        for case in cases {
            assert_eq!(
                PrStatus::derive(case.is_draft, case.review_decision),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn is_ordered_from_furthest_from_done_to_done() {
        assert!(PrStatus::Draft < PrStatus::AwaitingReview);
        assert!(PrStatus::AwaitingReview < PrStatus::ChangesRequested);
        assert!(PrStatus::ChangesRequested < PrStatus::Approved);
        // The maximum is the "best" PR — the one a Slice headline should follow.
        let statuses = [
            PrStatus::Draft,
            PrStatus::Approved,
            PrStatus::AwaitingReview,
        ];
        assert_eq!(statuses.iter().max().copied(), Some(PrStatus::Approved));
    }
}
