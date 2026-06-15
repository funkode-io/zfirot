//! The viewer-repositories GraphQL payload-to-`Project` projection, tested
//! offline against a recorded response fixture (`tests/fixtures/projects.json`).
//! The HTTP call is exercised manually; the parsing and cursor handling are
//! pinned here.

use infrastructure::parse_projects_response;

const PROJECTS_FIXTURE: &str = include_str!("fixtures/projects.json");

#[test]
fn parses_repositories_into_projects() {
    let (projects, next) = parse_projects_response(PROJECTS_FIXTURE).expect("fixture should parse");

    // The fixture is a single, final page.
    assert_eq!(next, None);

    // Four nodes collapse to three projects: the personal fork of zfirot and the
    // upstream funkode-io/zfirot de-duplicate onto the tracked upstream.
    assert_eq!(projects.len(), 3);

    // A non-fork repo carries its own owner/name and push timestamp straight
    // through; parsing does not reorder (the service owns recency ordering).
    let replay = &projects[0];
    assert_eq!(replay.repo.to_string(), "funkode-io/replay");
    assert_eq!(replay.pushed_at, "2024-04-15T09:30:00Z");

    // A fork stands in for its upstream parent: the personal carlos-verdes/zfirot
    // fork is tracked as funkode-io/zfirot, and de-duplicates with the directly
    // visible upstream, keeping the most recent push (the parent's).
    let zfirot: Vec<_> = projects
        .iter()
        .filter(|p| p.repo.name == "zfirot")
        .collect();
    assert_eq!(
        zfirot.len(),
        1,
        "the fork and upstream collapse to one project"
    );
    assert_eq!(zfirot[0].repo.to_string(), "funkode-io/zfirot");
    assert_eq!(zfirot[0].pushed_at, "2024-05-01T12:00:00Z");

    // A never-pushed repository reports a null `pushedAt`; it maps to an empty
    // string, which sorts last (least recent).
    let dotfiles = projects
        .iter()
        .find(|p| p.repo.name == "dotfiles")
        .expect("dotfiles repo present");
    assert_eq!(dotfiles.pushed_at, "");
}
