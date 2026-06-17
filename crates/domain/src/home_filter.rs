use crate::{Project, RepoRef};

/// The outcome of typing a query into the home-screen search box, decided purely
/// from the query and the currently-discovered projects.
///
/// Matching is a case-insensitive substring test on each project's `owner/name`
/// display string. The "Go to" action is *gated*: it only appears when nothing
/// matches yet the query is a valid `owner/repo`, so a repo already on the home
/// screen is opened by clicking its card, never by a redundant button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeFilter {
    /// One or more discovered projects match the query; show only these.
    Filtered(Vec<Project>),
    /// No project matches, but the query parses as a valid `owner/repo`; offer a
    /// single "Go to" action to open it directly.
    GoTo(RepoRef),
    /// No project matches and the query is not a valid repo path; show a quiet
    /// hint and offer no action.
    Hint,
}

/// Decide what the home screen shows for `query` over the discovered `projects`.
///
/// Matching is a case-insensitive substring test on each project's `owner/name`
/// display string. An empty or whitespace-only query matches every project, so
/// the default view lists all discovered projects.
///
/// ASCII case-folding is sufficient: `RepoRef::parse` constrains owner/name to
/// ASCII, so a project's display string is always ASCII. The empty-query path —
/// the common default state — skips per-project case-folding entirely.
pub fn filter_home(query: &str, projects: &[Project]) -> HomeFilter {
    let trimmed = query.trim();
    let matches: Vec<Project> = if trimmed.is_empty() {
        projects.to_vec()
    } else {
        let needle = trimmed.to_ascii_lowercase();
        projects
            .iter()
            .filter(|p| p.repo.to_string().to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect()
    };

    if !matches.is_empty() {
        return HomeFilter::Filtered(matches);
    }

    match RepoRef::parse(query) {
        Ok(repo) => HomeFilter::GoTo(repo),
        Err(_) => HomeFilter::Hint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(owner: &str, name: &str) -> Project {
        Project::new(RepoRef::new(owner, name), "2026-01-01T00:00:00Z")
    }

    fn projects() -> Vec<Project> {
        vec![
            project("funkode-io", "zfirot"),
            project("funkode-io", "retail-browser"),
            project("octocat", "hello-world"),
        ]
    }

    /// The matched repos as `owner/name` strings, for terse assertions.
    fn matched(outcome: &HomeFilter) -> Vec<String> {
        match outcome {
            HomeFilter::Filtered(ps) => ps.iter().map(|p| p.repo.to_string()).collect(),
            other => panic!("expected Filtered, got {other:?}"),
        }
    }

    #[test]
    fn empty_query_matches_every_project() {
        let ps = projects();
        assert_eq!(
            matched(&filter_home("", &ps)),
            vec![
                "funkode-io/zfirot",
                "funkode-io/retail-browser",
                "octocat/hello-world"
            ],
        );
    }

    #[test]
    fn whitespace_query_matches_every_project() {
        let ps = projects();
        assert_eq!(matched(&filter_home("   ", &ps)).len(), 3);
    }

    #[test]
    fn substring_on_owner_filters_to_matches() {
        let ps = projects();
        assert_eq!(
            matched(&filter_home("funkode", &ps)),
            vec!["funkode-io/zfirot", "funkode-io/retail-browser"],
        );
    }

    #[test]
    fn substring_on_name_filters_to_matches() {
        let ps = projects();
        assert_eq!(
            matched(&filter_home("retail", &ps)),
            vec!["funkode-io/retail-browser"]
        );
    }

    #[test]
    fn substring_spanning_owner_and_name_matches() {
        let ps = projects();
        // "io/z" spans the slash in the `owner/name` display string.
        assert_eq!(
            matched(&filter_home("io/z", &ps)),
            vec!["funkode-io/zfirot"]
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let ps = projects();
        assert_eq!(
            matched(&filter_home("OCTOCAT", &ps)),
            vec!["octocat/hello-world"]
        );
    }

    #[test]
    fn zero_matches_with_valid_repo_path_offers_goto() {
        let ps = projects();
        assert_eq!(
            filter_home("someone/elsewhere", &ps),
            HomeFilter::GoTo(RepoRef::new("someone", "elsewhere")),
        );
    }

    #[test]
    fn goto_preserves_owner_repo_case() {
        let ps = projects();
        assert_eq!(
            filter_home("SomeOne/Elsewhere", &ps),
            HomeFilter::GoTo(RepoRef::new("SomeOne", "Elsewhere")),
        );
    }

    #[test]
    fn zero_matches_with_invalid_repo_path_shows_hint() {
        let ps = projects();
        // No slash, so it is not a valid `owner/repo`.
        assert_eq!(filter_home("nonsense", &ps), HomeFilter::Hint);
    }

    #[test]
    fn a_discovered_repo_typed_in_full_stays_filtered_not_goto() {
        let ps = projects();
        // It matches a discovered project, so it is Filtered (card click), never
        // a redundant GoTo button.
        assert_eq!(
            matched(&filter_home("funkode-io/zfirot", &ps)),
            vec!["funkode-io/zfirot"],
        );
    }

    #[test]
    fn no_projects_and_empty_query_is_a_hint() {
        assert_eq!(filter_home("", &[]), HomeFilter::Hint);
    }
}
