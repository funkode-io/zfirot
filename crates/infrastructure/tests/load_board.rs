//! Smoke test: the load-board use-case runs end-to-end against the fake port.

use application::BoardService;
use domain::{RepoRef, SliceState};
use infrastructure::FakeGitHubPort;

#[tokio::test]
async fn load_board_returns_slices_across_states() {
    let service = BoardService::new(FakeGitHubPort);
    let repo = RepoRef::new("funkode-io", "zfirot");

    let slices = service
        .load_board(&repo)
        .await
        .expect("fake port should load the board");

    assert!(!slices.is_empty(), "expected canned slices");
    assert!(slices.iter().any(|s| s.state == SliceState::Ready));
    assert!(slices.iter().any(|s| s.state == SliceState::Wip));
    assert!(slices.iter().any(|s| s.state == SliceState::Blocked));
}
