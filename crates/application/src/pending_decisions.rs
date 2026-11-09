//! The Confirm and Veto decisions the board has not caught up with yet.

use domain::{IssueClassification, RepoRef};

use crate::ClassifiedBoard;

/// The issues a decision is outstanding on, per repo.
///
/// A decision is outstanding from the click until the board on screen stops
/// suggesting the issue. That outlasts the mutation on purpose: while the board
/// still shows the guess, a second click would add the opposite label, and since
/// `prd` beats `not-a-prd` it could silently undo an accepted Veto. So neither a
/// failed refresh nor a successful one returning the old board ends a decision.
///
/// Session state for the buttons, never persisted: the decision itself lives only
/// in the GitHub label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingDecisions(Vec<(RepoRef, u64)>);

impl PendingDecisions {
    /// Start a decision on an issue. Returns `false` when one is already
    /// outstanding, in which case the caller must not add a label.
    pub fn begin(&mut self, repo: &RepoRef, issue_number: u64) -> bool {
        if self.issues_in(repo).contains(&issue_number) {
            return false;
        }
        self.0.push((repo.clone(), issue_number));
        true
    }

    /// GitHub refused the label, so nothing changed and the user may try again.
    pub fn rejected(&mut self, repo: &RepoRef, issue_number: u64) {
        self.0.retain(|held| held != &(repo.clone(), issue_number));
    }

    /// End every decision in `repo` that `board`, the one on screen, has caught up
    /// with. Other repos are left alone: their boards are not on screen.
    pub fn settle(&mut self, repo: &RepoRef, board: &ClassifiedBoard) {
        self.0.retain(|(held_repo, number)| {
            held_repo != repo || still_suggested(board, *number)
        });
    }

    /// The issues in `repo` with a decision outstanding.
    pub fn issues_in(&self, repo: &RepoRef) -> Vec<u64> {
        self.0
            .iter()
            .filter(|(held_repo, _)| held_repo == repo)
            .map(|(_, number)| *number)
            .collect()
    }
}

/// Whether `board` still offers a decision on this issue: heading a suggested Lane,
/// or as a suggestion in the tray.
fn still_suggested(board: &ClassifiedBoard, number: u64) -> bool {
    board
        .slices
        .iter()
        .filter_map(|slice| slice.prd.as_ref())
        .any(|prd| prd.number == number && prd.suggested)
        || board.other.iter().any(|issue| {
            issue.number == number
                && matches!(
                    issue.classification,
                    IssueClassification::SuggestedPrd | IssueClassification::SuggestedSlice
                )
        })
}
