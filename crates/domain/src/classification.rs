use serde::{Deserialize, Serialize};

/// How an open GitHub issue has been classified by the two-tier strategy.
///
/// - **Tier 1 (confident, automatic):** `prd` label → [`IssueClassification::Prd`];
///   native child of a PRD or `slice`/`ready-for-agent` label →
///   [`IssueClassification::Slice`].
/// - **Tier 2 (heuristic, suggested):** unlabeled issues scored against the
///   planning-skill template headings → [`IssueClassification::SuggestedPrd`] or
///   [`IssueClassification::SuggestedSlice`].
/// - **Tier 3:** nothing matches → [`IssueClassification::Unclassified`], shown
///   in "other open issues".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueClassification {
    /// Confident — carries the `prd` label.
    Prd,
    /// Confident — native child of a PRD, or carries `slice`/`ready-for-agent`.
    Slice,
    /// Heuristic — body matches PRD template headings; unconfirmed.
    SuggestedPrd,
    /// Heuristic — body matches Slice template headings; unconfirmed.
    SuggestedSlice,
    /// No match — shown in the "other open issues" bucket with no suggestion.
    Unclassified,
}

impl IssueClassification {
    /// The label a confirm action should add to promote a *suggested* issue to a
    /// confident tier-1 classification: `prd` for [`IssueClassification::SuggestedPrd`]
    /// and `slice` for [`IssueClassification::SuggestedSlice`].
    ///
    /// Returns `None` for already-confident or unclassified issues, which have
    /// no suggestion to confirm.
    pub fn suggested_label(&self) -> Option<&'static str> {
        match self {
            IssueClassification::SuggestedPrd => Some("prd"),
            IssueClassification::SuggestedSlice => Some("slice"),
            IssueClassification::Prd
            | IssueClassification::Slice
            | IssueClassification::Unclassified => None,
        }
    }
}

/// Raw, GitHub-shaped facts about a single issue before classification.
///
/// An adapter projects GitHub API data into this type. The pure
/// [`classify_issue`] function and the prose-fallback parsing utilities
/// (`parse_parent_from_body`, `parse_blockers_from_body`) then operate on it.
///
/// It deliberately carries **no Pull Request data**: Linked PRs are read from
/// the PR side by an open-PR **Sweep** ([`crate::RawLinkedPr`]) and joined onto
/// Slices at classification time, so the issue queries stay shallow and "a
/// Linked PR is an *open* PR" holds by construction (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawIssue {
    /// The GitHub issue number.
    pub number: u64,
    pub title: String,
    /// The issue's URL on GitHub, for opening it in a browser.
    pub url: String,
    /// Raw Markdown body of the issue.
    pub body: Option<String>,
    /// Labels applied to the issue (e.g. `"prd"`, `"slice"`, `"ready-for-agent"`).
    pub labels: Vec<String>,
    /// `true` when the issue is closed.
    pub closed: bool,
    /// Issue number of the native GitHub sub-issue parent, if present.
    pub native_parent: Option<u64>,
    /// Issue numbers of native "blocked by" dependency issues (open and closed).
    pub native_blockers: Vec<u64>,
    /// GitHub login of the assignee, when assigned.
    pub assignee: Option<String>,
    /// Avatar URL of the assignee, when assigned and available.
    pub assignee_avatar_url: Option<String>,
    /// `true` when this issue is a native sub-issue child of an issue that
    /// carries the `prd` label.
    pub is_native_child_of_prd: bool,
}

/// Classify a raw issue using the two-tier strategy plus an unclassified fallback.
///
/// This is a pure function: it reads only the fields of `raw` and is therefore
/// testable without GitHub access.
///
/// ### Classification order
///
/// 1. **Tier 1 – confident:** `prd` label → [`IssueClassification::Prd`];
///    `is_native_child_of_prd` or `slice`/`ready-for-agent` label →
///    [`IssueClassification::Slice`].
/// 2. **Tier 2 – heuristic:** score the body against planning-skill template
///    headings: *Problem Statement* + *User Stories* → [`IssueClassification::SuggestedPrd`];
///    *What to build* + (*Acceptance criteria* | *Blocked by* | *Parent*) →
///    [`IssueClassification::SuggestedSlice`].
/// 3. **Tier 3:** [`IssueClassification::Unclassified`].
pub fn classify_issue(raw: &RawIssue) -> IssueClassification {
    // Tier 1: confident, based on labels or native parent link.
    if raw.labels.iter().any(|l| l == "prd") {
        return IssueClassification::Prd;
    }
    if raw.is_native_child_of_prd
        || raw
            .labels
            .iter()
            .any(|l| l == "slice" || l == "ready-for-agent")
    {
        return IssueClassification::Slice;
    }

    // Tier 2: heuristic, scored from template headings in the issue body.
    let body = raw.body.as_deref().unwrap_or("");
    if has_prd_headings(body) {
        return IssueClassification::SuggestedPrd;
    }
    if has_slice_headings(body) {
        return IssueClassification::SuggestedSlice;
    }

    IssueClassification::Unclassified
}

/// Parse the parent issue number from the `## Parent` section of an issue body.
///
/// This reads prose only. Callers read the native parent link first and use
/// this as a fallback when that link is absent. Returns `None` if no
/// `#<number>` reference is found in the section.
pub fn parse_parent_from_body(body: &str) -> Option<u64> {
    let section = extract_section_ci(body, "## parent")?;
    first_issue_ref(section)
}

/// Parse "blocked by" issue numbers from the `## Blocked by` section of an
/// issue body.
///
/// This reads prose only. Callers read the native dependency links first and
/// use this as a fallback when those links are absent. Returns an empty `Vec`
/// when the section is missing or contains no `#<number>` references.
pub fn parse_blockers_from_body(body: &str) -> Vec<u64> {
    let Some(section) = extract_section_ci(body, "## blocked by") else {
        return Vec::new();
    };
    section
        .lines()
        .filter_map(|line| {
            let stripped =
                line.trim_start_matches(|c: char| c == '-' || c == '*' || c.is_whitespace());
            first_issue_ref(stripped)
        })
        .collect()
}

/// Whether an issue body has a `## Blocked by` section that names **no** issue
/// yet still states a wait — the **External blocker** signal.
///
/// The planning skills emit this section for every Slice, so prose in it ("a
/// prod values file existing for this app") is a real, stated wait on something
/// GitHub cannot link. Three shapes are *not* the signal:
///
/// - A section naming issues, whether those issues are open, closed, or outside
///   the fetched board: the references speak for themselves, and treating a
///   satisfied dependency as an external wait would strand a Slice that has
///   become workable.
/// - A section holding only a "no blockers" placeholder ("None — can start
///   immediately."), which the same skills emit for every unblocked Slice and
///   which asserts the opposite of a wait.
/// - An empty section, which is no evidence of anything.
pub fn has_unreferenced_blocked_by(body: &str) -> bool {
    let Some(section) = extract_section_ci(body, "## blocked by") else {
        return false;
    };
    if !parse_blockers_from_body(body).is_empty() {
        return false;
    }
    section.lines().any(states_a_wait)
}

/// Whether one line of a `## Blocked by` section states a wait, as opposed to
/// being blank or a "no blockers" placeholder (`None`, `N/A`, and the phrases
/// they lead: "None — can start immediately.").
///
/// A placeholder is recognised by its opening word only, so "Nonexistent prod
/// values file" still reads as a wait. Prose that merely *starts* "None ..."
/// is read as a placeholder: leaving an ambiguous line in Ready matches the
/// behaviour before this signal existed.
fn states_a_wait(line: &str) -> bool {
    let text = line.trim_start_matches(|c: char| c == '-' || c == '*' || c.is_whitespace());
    let lower = text.to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    !["none", "n/a"].iter().any(|placeholder| {
        lower
            .strip_prefix(placeholder)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()))
    })
}

// ── Heuristic heading matchers ───────────────────────────────────────────────

fn has_prd_headings(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("## problem statement") && lower.contains("## user stories")
}

fn has_slice_headings(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("## what to build")
        && (lower.contains("## acceptance criteria")
            || lower.contains("## blocked by")
            || lower.contains("## parent"))
}

// ── Prose-parsing helpers ────────────────────────────────────────────────────

/// Extract the text of the first section whose heading matches `heading_lower`
/// (case-insensitive). Returns the trimmed content between that heading and the
/// next `##`-level heading (or the end of the body).
fn extract_section_ci<'a>(body: &'a str, heading_lower: &str) -> Option<&'a str> {
    // Lowercase the body for position-stable case-insensitive search.
    // `to_ascii_lowercase` is length-preserving, so byte positions are valid
    // indices into the original `body` slice.
    let body_lower = body.to_ascii_lowercase();
    let pos = body_lower.find(heading_lower)?;
    let after_heading = &body[pos + heading_lower.len()..];
    // Section ends at the next `##` heading or end of string.
    let end = after_heading.find("\n##").unwrap_or(after_heading.len());
    Some(after_heading[..end].trim())
}

/// Return the first `#<number>` issue reference found in `text`, or `None`.
fn first_issue_ref(text: &str) -> Option<u64> {
    let hash_pos = text.find('#')?;
    let after = &text[hash_pos + 1..];
    let num_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    if num_end == 0 {
        return None;
    }
    after[..num_end].parse().ok()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal raw issue with no labels, no body, and no native links —
    /// a blank slate for table-driven tests.
    fn blank_raw(number: u64) -> RawIssue {
        RawIssue {
            number,
            title: format!("Issue {number}"),
            url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
            body: None,
            labels: Vec::new(),
            closed: false,
            native_parent: None,
            native_blockers: Vec::new(),
            assignee: None,
            assignee_avatar_url: None,
            is_native_child_of_prd: false,
        }
    }

    // ── Tier 1: confident classification ─────────────────────────────────────

    #[test]
    fn prd_label_classifies_as_prd() {
        let raw = RawIssue {
            labels: vec!["prd".to_string()],
            ..blank_raw(1)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Prd);
    }

    #[test]
    fn slice_label_classifies_as_slice() {
        let raw = RawIssue {
            labels: vec!["slice".to_string()],
            ..blank_raw(2)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Slice);
    }

    #[test]
    fn ready_for_agent_label_classifies_as_slice() {
        let raw = RawIssue {
            labels: vec!["ready-for-agent".to_string()],
            ..blank_raw(3)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Slice);
    }

    #[test]
    fn native_child_of_prd_classifies_as_slice() {
        let raw = RawIssue {
            is_native_child_of_prd: true,
            ..blank_raw(4)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Slice);
    }

    #[test]
    fn prd_label_takes_precedence_over_native_child() {
        // An issue can theoretically carry `prd` AND be a native child; `prd` wins.
        let raw = RawIssue {
            labels: vec!["prd".to_string()],
            is_native_child_of_prd: true,
            ..blank_raw(5)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Prd);
    }

    // ── Tier 2: heuristic classification ─────────────────────────────────────

    #[test]
    fn prd_template_headings_suggest_prd() {
        let body = "## Problem Statement\n\nSome problem.\n\n## User Stories\n\n- As a user…";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(10)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::SuggestedPrd);
    }

    #[test]
    fn slice_template_headings_suggest_slice() {
        let body =
            "## What to build\n\nBuild the thing.\n\n## Acceptance criteria\n\n- [ ] It works";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(11)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::SuggestedSlice);
    }

    #[test]
    fn slice_headings_with_blocked_by_suggest_slice() {
        let body = "## What to build\n\nBuild it.\n\n## Blocked by\n\n- #3";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(12)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::SuggestedSlice);
    }

    #[test]
    fn slice_headings_with_parent_suggest_slice() {
        let body = "## What to build\n\nBuild it.\n\n## Parent\n\n#1";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(13)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::SuggestedSlice);
    }

    #[test]
    fn prd_headings_require_both_sections() {
        // Only "Problem Statement" → not a suggested PRD (no "User Stories").
        let body = "## Problem Statement\n\nSome problem.";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(14)
        };
        assert_ne!(classify_issue(&raw), IssueClassification::SuggestedPrd);
    }

    #[test]
    fn slice_headings_require_what_to_build() {
        // Only "Acceptance criteria" without "What to build" → not a suggested Slice.
        let body = "## Acceptance criteria\n\n- [ ] It works";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(15)
        };
        assert_ne!(classify_issue(&raw), IssueClassification::SuggestedSlice);
    }

    #[test]
    fn tier1_takes_precedence_over_heuristic() {
        // `slice` label + PRD body headings → Slice (tier 1 wins).
        let body = "## Problem Statement\n\nX.\n\n## User Stories\n\nY.";
        let raw = RawIssue {
            labels: vec!["slice".to_string()],
            body: Some(body.to_string()),
            ..blank_raw(20)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Slice);
    }

    // ── Tier 3: unclassified ──────────────────────────────────────────────────

    #[test]
    fn no_match_is_unclassified() {
        let raw = blank_raw(99);
        assert_eq!(classify_issue(&raw), IssueClassification::Unclassified);
    }

    #[test]
    fn body_with_unrelated_headings_is_unclassified() {
        let body = "## Summary\n\nJust a description.";
        let raw = RawIssue {
            body: Some(body.to_string()),
            ..blank_raw(100)
        };
        assert_eq!(classify_issue(&raw), IssueClassification::Unclassified);
    }

    // ── Confirming a suggestion: suggested_label ──────────────────────────────

    #[test]
    fn suggested_classifications_map_to_their_confirming_label() {
        let cases = [
            (IssueClassification::SuggestedPrd, Some("prd")),
            (IssueClassification::SuggestedSlice, Some("slice")),
            // Already-confident or unclassified issues have nothing to confirm.
            (IssueClassification::Prd, None),
            (IssueClassification::Slice, None),
            (IssueClassification::Unclassified, None),
        ];
        for (classification, expected) in cases {
            assert_eq!(
                classification.suggested_label(),
                expected,
                "{classification:?} should map to {expected:?}"
            );
        }
    }

    // ── Prose-fallback: has_unreferenced_blocked_by ───────────────────────────

    /// A `## Blocked by` section that names no issue is a real wait on something
    /// the dependency graph cannot see (dxp-data-loom#1250: "a prod values file
    /// existing for this app"). A section that *does* name issues is not — those
    /// references speak for themselves, whether they are open, closed, or absent
    /// from the fetched board. Nor is the "no blockers" placeholder the planning
    /// skills emit for every unblocked Slice, which asserts the opposite of a
    /// wait.
    #[test]
    fn detects_a_blocked_by_section_that_names_no_issue() {
        let cases: [(&str, bool, &str); 12] = [
            (
                "## Blocked by\n\n- A prod values file / prod environment existing for this app.\n",
                true,
                "prose naming no issue",
            ),
            (
                "## What to build\n\nStuff\n\n## Blocked by\n\nWaiting on the platform team.\n\n## Notes\n\nx",
                true,
                "prose section between other sections",
            ),
            (
                "## Blocked by\n\n- #5\n",
                false,
                "names an issue",
            ),
            (
                "## Blocked by\n\n- funkode-io/zfirot#5 — and also a prod environment\n",
                false,
                "names an issue alongside prose",
            ),
            (
                "## What to build\n\nStuff\n",
                false,
                "no section at all",
            ),
            (
                "## Blocked by\n\n## Acceptance criteria\n\n- [ ] x",
                false,
                "an empty section is no evidence of a wait",
            ),
            (
                "## Blocked by\n\n- None — can start immediately.\n",
                false,
                "the skills' bulleted no-blockers placeholder",
            ),
            (
                "## Blocked by\n\nNone - can start immediately\n",
                false,
                "the same placeholder unbulleted, with an ASCII dash",
            ),
            (
                "## Blocked by\n\nNone.\n",
                false,
                "a bare None",
            ),
            (
                "## Blocked by\n\nN/A\n",
                false,
                "N/A is the same assertion",
            ),
            (
                "## Blocked by\n\n- Nonexistent prod values file for this app.\n",
                true,
                "a wait whose first word merely starts with 'none'",
            ),
            (
                "## Blocked by\n\n- None of this is the point.\n- Waiting on the platform team.\n",
                true,
                "one placeholder line does not cancel a stated wait",
            ),
        ];

        for (body, expected, name) in cases {
            assert_eq!(has_unreferenced_blocked_by(body), expected, "{name}");
        }
    }

    // ── Prose-fallback: parse_parent_from_body ────────────────────────────────

    #[test]
    fn parse_parent_finds_owner_repo_ref() {
        let body = "## Parent\n\nfunkode-io/zfirot#1\n\n## Other\n\ntext";
        assert_eq!(parse_parent_from_body(body), Some(1));
    }

    #[test]
    fn parse_parent_finds_bare_ref() {
        let body = "## Parent\n\n#42\n";
        assert_eq!(parse_parent_from_body(body), Some(42));
    }

    #[test]
    fn parse_parent_case_insensitive_heading() {
        let body = "## PARENT\n\n#7\n";
        assert_eq!(parse_parent_from_body(body), Some(7));
    }

    #[test]
    fn parse_parent_returns_none_when_section_absent() {
        let body = "## What to build\n\nStuff";
        assert_eq!(parse_parent_from_body(body), None);
    }

    #[test]
    fn parse_parent_returns_none_when_no_ref_in_section() {
        let body = "## Parent\n\nSee the main issue.\n";
        assert_eq!(parse_parent_from_body(body), None);
    }

    // ── Prose-fallback: parse_blockers_from_body ──────────────────────────────

    #[test]
    fn parse_blockers_finds_multiple_refs() {
        let body = "## Blocked by\n\n- funkode-io/zfirot#3\n- #7\n\n## Next";
        let blockers = parse_blockers_from_body(body);
        assert_eq!(blockers, vec![3, 7]);
    }

    #[test]
    fn parse_blockers_finds_single_ref() {
        let body = "## Blocked by\n\n- #5\n";
        assert_eq!(parse_blockers_from_body(body), vec![5]);
    }

    #[test]
    fn parse_blockers_case_insensitive_heading() {
        let body = "## BLOCKED BY\n\n- #9\n";
        assert_eq!(parse_blockers_from_body(body), vec![9]);
    }

    #[test]
    fn parse_blockers_returns_empty_when_section_absent() {
        let body = "## Parent\n\n#1\n";
        assert!(parse_blockers_from_body(body).is_empty());
    }

    #[test]
    fn parse_blockers_returns_empty_when_no_refs_in_section() {
        let body = "## Blocked by\n\nNone at this time.\n";
        assert!(parse_blockers_from_body(body).is_empty());
    }
}
