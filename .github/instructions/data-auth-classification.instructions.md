---
description: GitHub GraphQL data source, PAT auth, freshness, and issue classification
applyTo: "crates/{infrastructure,application}/**/*.rs"
---

# Data, Auth & Classification

## Data & auth

- Data source: GitHub **GraphQL** only. The board is read as two collections:
  one query returns a project's issues with parent, dependencies, assignees and
  labels (and **no** Pull Request data), and an open-PR **Sweep** returns every
  open PR with its status, Decorations, and the issue numbers it closes (or,
  with no closing reference, relates to in prose). The pure `classify` joins
  them by issue number. Mutations: assign self
  (`addAssigneesToAssignable`) and add a classifying label.
- Auth: a user-supplied fine-grained **Personal Access Token** stored in the OS
  secure store. Scopes: Issues read/write, Pull requests read, Contents read.
- Freshness: manual Refresh + a configurable background poll (default ~60s) +
  a "last updated" timestamp. That timestamp is **when the shown facts were
  fetched**, never when they were painted: a board painted from the cache is
  stamped with the snapshot's `fetched_at`, or stale-while-revalidate silently
  presents old facts as current.

### GitHub API traps (each one cost a bug)

- Read the PR→issue edge **from the PR side** (`closingIssuesReferences` on an
  `states: [OPEN]` PR sweep), never from the issue side. An issue's
  `closedByPullRequestsReferences(includeClosedPrs: false)` is **not** "open PRs
  only": it drops CLOSED-unmerged references but still returns **MERGED** ones,
  which is how a merged PR held a Slice in WIP forever (#161, #179). Sweeping
  open PRs makes "a Linked PR is an *open* PR" true by construction, and keeps
  the issue queries shallow enough never to approach GitHub's node limit (#168).
- `closingIssuesReferences` is filled in **only for a PR targeting the default
  branch**. A Stacked PR reports `[]` until the PR beneath it merges and GitHub
  retargets it, whatever its body says, so a PR with no closing reference falls
  back to the closing keywords in its body (#196).
- An issue's `updatedAt` does **not** advance when a PR gains or changes a
  closing reference to it, so the issue index can never see a new Linked PR. That
  is why the open-PR sweep runs on its own hot loop (and on cold load and cached
  open), never derived from the issue side. The issue-side paths (structural,
  safety net) write only issues and adopt the hot path's PR facts from the cache.
- The issue-index sweep is **authoritative**: an issue absent from it is dropped,
  so its parser must fail on anything short of a complete page rather than let a
  truncated answer read as "gone". Never answer it from a `since`-style change
  feed: GitHub cannot report a deletion or a transfer through one.
  A change to which PRs close which issues **Cascades**: the hot path hydrates
  those issues by number in the same operation, so it is the one hot write that
  touches issues, and the issue-side paths keep it rather than reverting it.
- Facts that a safety-net hydrate re-fetches identically can never be healed by
  more refresh plumbing. When a value looks stale after a safety net, suspect the
  query or the projection, not the cache.
- The board snapshot's stored format carries a version. Change its shape and
  bump the version, so older cached snapshots are discarded and cold-loaded
  rather than mis-read.

## Classification (two-tier)

- Tier 1 (confident, automatic): `prd` label -> PRD; native PRD parent or
  `slice`/`ready-for-agent` label -> Slice.
- Tier 2 (heuristic, suggested): unlabeled issues scored by the planning-skill
  template headings (Problem Statement + User Stories -> PRD; What to build +
  Acceptance criteria / Blocked by / Parent -> Slice), surfaced with a
  "looks like a PRD/Slice — confirm?" badge and a confirm-and-add-label action.
- Tier 3: no match -> Unclassified, shown inline in "other open issues".
- Second pass (ADR 0007): an issue left at Suggested Slice or tier 3 becomes a Slice when its
  Parent (native, else prose) is an open PRD, Suggested PRD or Slice, or is closed or
  outside the fetched board ("No PRD" Lane). A Parent that is any other open issue
  promotes nothing.
- A Suggested PRD heads a Lane marked unconfirmed. Its evidence is its headings,
  so editing them away drops the Lane: a prose `## Parent` names a Lane only while
  it resolves to a PRD, Suggested PRD or Slice. A labelled Slice with a native link
  keeps its Lane whatever the parent is.
