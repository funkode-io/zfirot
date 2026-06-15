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
}
