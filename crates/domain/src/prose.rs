//! Pure parsing of the `## Parent` / `## Blocked by` prose sections that the
//! planning skills emit in an issue body.
//!
//! This is the fallback used when GitHub's native sub-issue (`parent`) and
//! dependency (`blockedBy`) links are absent: the relationships still exist as
//! prose in the body, so the board parses them here and resolves the referenced
//! numbers against the issues it already fetched.

/// Issue references parsed from an issue body's relationship sections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProseLinks {
    /// The parent (PRD) issue number from a `## Parent` section, if any.
    pub parent: Option<u64>,
    /// The blocker issue numbers from a `## Blocked by` section, in order and
    /// de-duplicated.
    pub blocked_by: Vec<u64>,
}

/// Parse the `## Parent` and `## Blocked by` sections of an issue body.
///
/// Headings are matched case-insensitively and a section runs until the next
/// Markdown heading. References may be `owner/repo#123`, `#123`, or an
/// `.../issues/123` URL. The parent is the first reference found in `## Parent`;
/// blockers are every reference in `## Blocked by`, de-duplicated in order.
pub fn parse_prose(body: &str) -> ProseLinks {
    let mut links = ProseLinks::default();
    let mut section = Section::None;

    for line in body.lines() {
        if let Some(title) = heading_title(line) {
            section = Section::from_title(title);
            continue;
        }

        match section {
            Section::Parent => {
                if links.parent.is_none() {
                    if let Some(&number) = issue_numbers_in(line).first() {
                        links.parent = Some(number);
                    }
                }
            }
            Section::BlockedBy => {
                for number in issue_numbers_in(line) {
                    if !links.blocked_by.contains(&number) {
                        links.blocked_by.push(number);
                    }
                }
            }
            Section::None | Section::Other => {}
        }
    }

    links
}

/// The phrases that tie a pull request to an issue without closing it, longest
/// first so `related to` wins over `related` at the same position.
const RELATION_PHRASES: [&str; 8] = [
    "relating to",
    "related to",
    "relates to",
    "references",
    "part of",
    "related",
    "refs",
    "ref",
];

/// Parse the issues a pull request says it relates to without closing them:
/// `Related to #12`, `relates to issue 12`, `Part of #12`, `Refs: #12, #13`.
///
/// This reads prose only. GitHub records no closing reference for these, so
/// callers read a PR's closing references first and use this as the fallback
/// when there are none.
///
/// A relation and its references must share a line, so a PR title ending in
/// `refs` or `not` says nothing about a body joined after it. Only an explicit
/// relation counts: a bare `#12` mention, `unrelated to #12`
/// and `not related to #12` do not. Only same-repo `#N` forms count, since
/// `owner/repo#12` or a URL may name another repository's issue. Numbers are
/// de-duplicated, in order.
pub fn parse_related_issues(text: &str) -> Vec<u64> {
    // ASCII lowercasing keeps every byte offset valid for slicing.
    let lowered = text.to_ascii_lowercase();
    let mut numbers = Vec::new();

    for line in lowered.lines() {
        for (start, _) in line.char_indices() {
            let rest = &line[start..];
            let Some(phrase) = RELATION_PHRASES.iter().find(|p| rest.starts_with(**p)) else {
                continue;
            };
            let before = &line[..start];
            // A relation opens its whitespace-separated word, so one inside a
            // path, a URL or a longer word (`other/refs#12`, `?q=refs#12`,
            // `unrelated`) never counts.
            let opens_a_word = before
                .rsplit(char::is_whitespace)
                .next()
                .is_some_and(|word| word.chars().all(is_wrapper));
            if !opens_a_word || is_negated(before) {
                continue;
            }
            for number in references_after_relation(&rest[phrase.len()..]) {
                if !numbers.contains(&number) {
                    numbers.push(number);
                }
            }
        }
    }

    numbers
}

/// Markdown and quoting that may wrap a word without changing it.
fn is_wrapper(c: char) -> bool {
    "([{*_\"'`~".contains(c)
}

/// Whether the text before a relation phrase ends in the word `not`, looking
/// through the Markdown and quoting around it: `(not`, `*not*`, `"not`. A `not.`
/// ends its own sentence, so it negates nothing after it.
fn is_negated(before: &str) -> bool {
    before
        .trim_end_matches(|c: char| c.is_whitespace() || is_wrapper(c))
        .split_whitespace()
        .next_back()
        .map(|word| word.trim_start_matches(is_wrapper))
        == Some("not")
}

/// The issue numbers directly after a relation phrase: an optional `issue` word
/// (which also makes the first `#` optional), then a list of references joined
/// by `,`, `&` or `and`. Anything else ends the list.
fn references_after_relation(text: &str) -> Vec<u64> {
    // Not `_`: `related_issue_12` is an identifier, not a relation.
    let is_gap = |c: char| c.is_whitespace() || c == ':' || c == '*';
    let mut rest = text.trim_start_matches(is_gap);
    let mut hash_required = true;
    for word in ["issues", "issue"] {
        // `issue 12` or `issue#12`, but `issues12` is just a word.
        if let Some(after_word) = rest
            .strip_prefix(word)
            .filter(|after| after.starts_with(is_gap) || after.starts_with('#'))
        {
            rest = after_word.trim_start_matches(is_gap);
            hash_required = false;
            break;
        }
    }

    let mut numbers = Vec::new();
    loop {
        let reference = match rest.strip_prefix('#') {
            Some(after_hash) => after_hash,
            None if !hash_required => rest,
            None => break,
        };
        let Some(number) = leading_number(reference) else {
            break;
        };
        // `#1a2b` is a colour or a hash, not an issue reference.
        let after_number = reference.trim_start_matches(|c: char| c.is_ascii_digit());
        if after_number.starts_with(|c: char| c.is_alphanumeric()) {
            break;
        }
        numbers.push(number);

        let separator = after_number.trim_start();
        let Some(next) = ["and ", ",", "&"]
            .iter()
            .find_map(|joiner| separator.strip_prefix(joiner))
        else {
            break;
        };
        // A list's last item may be joined by both: `#1, #2, and #3`.
        let next = next.trim_start();
        rest = next
            .strip_prefix("and ")
            .or_else(|| next.strip_prefix('&'))
            .unwrap_or(next)
            .trim_start();
        hash_required = true;
    }
    numbers
}

/// Which relationship section the parser is currently inside.
enum Section {
    None,
    Parent,
    BlockedBy,
    Other,
}

impl Section {
    fn from_title(title: &str) -> Self {
        match title.trim().to_ascii_lowercase().as_str() {
            "parent" => Section::Parent,
            "blocked by" => Section::BlockedBy,
            _ => Section::Other,
        }
    }
}

/// If `line` is a Markdown ATX heading (`# ...`), return its title text.
///
/// A heading requires whitespace after the `#` run, which is what distinguishes
/// it from an issue reference like `#7`.
fn heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.trim_start_matches('#');
    if rest == trimmed {
        return None; // no leading '#'
    }
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None // e.g. `#7` is an issue reference, not a heading
    }
}

/// Extract every issue number referenced in `line` (`#123` or `/issues/123`).
fn issue_numbers_in(line: &str) -> Vec<u64> {
    let mut numbers = Vec::new();

    for part in line.split('#').skip(1) {
        if let Some(number) = leading_number(part) {
            numbers.push(number);
        }
    }

    for (index, marker) in line.match_indices("/issues/") {
        if let Some(number) = leading_number(&line[index + marker.len()..]) {
            numbers.push(number);
        }
    }

    numbers
}

/// Parse the run of ASCII digits at the start of `text`, if any.
fn leading_number(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parent_and_blocked_by_references() {
        struct Case {
            name: &'static str,
            body: &'static str,
            parent: Option<u64>,
            blocked_by: Vec<u64>,
        }

        let cases = [
            Case {
                name: "cross-repo parent and a bulleted blocked-by list",
                body: "## What to build\n\nDo the thing.\n\n\
                       ## Parent\n\nfunkode-io/zfirot#1\n\n\
                       ## Blocked by\n\n- funkode-io/zfirot#4\n- funkode-io/zfirot#5\n",
                parent: Some(1),
                blocked_by: vec![4, 5],
            },
            Case {
                name: "bare hash references",
                body: "## Parent\n#7\n## Blocked by\n#8\n",
                parent: Some(7),
                blocked_by: vec![8],
            },
            Case {
                name: "issue URL references",
                body: "## Parent\n\nhttps://github.com/funkode-io/zfirot/issues/3\n",
                parent: Some(3),
                blocked_by: vec![],
            },
            Case {
                name: "case-insensitive headings",
                body: "## PARENT\n\nfunkode-io/zfirot#2\n\n## blocked BY\n\n#9\n",
                parent: Some(2),
                blocked_by: vec![9],
            },
            Case {
                name: "missing sections yield nothing",
                body: "## What to build\n\nNo relationships here.\n",
                parent: None,
                blocked_by: vec![],
            },
            Case {
                name: "empty body",
                body: "",
                parent: None,
                blocked_by: vec![],
            },
            Case {
                name: "parent heading with no reference",
                body: "## Parent\n\n_None yet._\n",
                parent: None,
                blocked_by: vec![],
            },
            Case {
                name: "a later heading ends the section",
                body: "## Parent\n\nfunkode-io/zfirot#1\n\n## Acceptance criteria\n\n- close #99\n",
                parent: Some(1),
                blocked_by: vec![],
            },
            Case {
                name: "first reference wins for the parent",
                body: "## Parent\n\nfunkode-io/zfirot#1 (supersedes #2)\n",
                parent: Some(1),
                blocked_by: vec![],
            },
            Case {
                name: "duplicate blockers are de-duplicated in order",
                body: "## Blocked by\n\n- #4\n- #5\n- #4\n",
                parent: None,
                blocked_by: vec![4, 5],
            },
        ];

        for case in cases {
            let links = parse_prose(case.body);
            assert_eq!(links.parent, case.parent, "parent: {}", case.name);
            assert_eq!(
                links.blocked_by, case.blocked_by,
                "blocked_by: {}",
                case.name
            );
        }
    }

    #[test]
    fn parses_issues_a_pr_relates_to_without_closing_them() {
        let cases: [(&str, &str, Vec<u64>); 34] = [
            ("the reported wording", "Related to #12", vec![12]),
            ("any case", "RELATES TO #12.", vec![12]),
            ("the word issue", "related to issue #12", vec![12]),
            ("the word issue, no hash", "Related to issue 12", vec![12]),
            ("a colon", "Refs: #12", vec![12]),
            ("part of", "Part of #12", vec![12]),
            ("a list", "Related to #1, #2 and #3 & #4", vec![1, 2, 3, 4]),
            (
                "repeats once, in order",
                "Refs #2. Part of #1. Ref #2",
                vec![2, 1],
            ),
            (
                "a bare mention is no relation",
                "Follow-up to #12, see #13",
                vec![],
            ),
            (
                "a closing keyword is GitHub's to read",
                "Closes #12",
                vec![],
            ),
            ("inside another word", "Unrelated to #12, prefs #13", vec![]),
            ("negated", "This is not related to #12", vec![]),
            ("another repository", "Related to other/repo#12", vec![]),
            ("an unfilled template", "Related to #", vec![]),
            ("not an issue number", "refs #1a2b", vec![]),
            (
                "a list with a final comma",
                "Related to #1, #2, and #3",
                vec![1, 2, 3],
            ),
            (
                "a reference on the next line",
                "update refs\n#12 reproduces it",
                vec![],
            ),
            (
                "a not on the previous line",
                "Support SQL NOT\nRelated to #12",
                vec![12],
            ),
            ("a repository path", "Related to other/refs#12", vec![]),
            ("a URL", "https://example.com/refs#12", vec![]),
            (
                "the word issue run into a number",
                "Related to issues12",
                vec![],
            ),
            (
                "the word issue before a hash",
                "Related to issue#12",
                vec![12],
            ),
            ("an identifier", "add related_issue_12 fixture", vec![]),
            ("a short identifier", "add ref_issue_12 fixture", vec![]),
            (
                "the word issue in an identifier",
                "related to issue_12",
                vec![],
            ),
            (
                "a URL query value",
                "https://example.com/?q=refs#12",
                vec![],
            ),
            ("bold around the phrase", "**Related to:** #12", vec![12]),
            ("a phrase in parentheses", "(part of #12)", vec![12]),
            ("a hyphenated word", "cross-refs #12", vec![]),
            ("negated in parentheses", "(not related to #12)", vec![]),
            ("negated in quotes", "\"not related to #12\"", vec![]),
            (
                "negated with emphasis",
                "This is *not* related to #12",
                vec![],
            ),
            (
                "emphasis after the negation",
                "not *related to #12*",
                vec![],
            ),
            (
                "a not ending its sentence",
                "It is not. Related to #12",
                vec![12],
            ),
        ];

        for (name, text, expected) in cases {
            assert_eq!(parse_related_issues(text), expected, "{name}");
        }
    }
}
