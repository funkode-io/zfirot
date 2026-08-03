---
description: GitHub GraphQL data source, PAT auth, freshness, and issue classification
applyTo: "crates/{infrastructure,application}/**/*.rs"
---

# Data, Auth & Classification

## Data & auth

- Data source: GitHub **GraphQL** only. One query per project returns its issues
  with parent, dependencies, assignees, labels, and linked-PR state. Mutations:
  assign self (`addAssigneesToAssignable`) and add a classifying label.
- Auth: a user-supplied fine-grained **Personal Access Token** stored in the OS
  secure store. Scopes: Issues read/write, Pull requests read, Contents read.
- Freshness: manual Refresh + a configurable background poll (default ~60s) +
  a "last updated" timestamp. That timestamp is **when the shown facts were
  fetched**, never when they were painted: a board painted from the cache is
  stamped with the snapshot's `fetched_at`, or stale-while-revalidate silently
  presents old facts as current.

### GitHub API traps (each one cost a bug)

- `closedByPullRequestsReferences(includeClosedPrs: false)` is **not** "open PRs
  only": it drops CLOSED-unmerged references but still returns **MERGED** ones.
  A Linked PR is an *open* PR, so always select the PR's own `state` and keep
  only `OPEN`. Merged references also consume page slots, so page past the
  handful an issue is likely to have.
- An issue's `updatedAt` does **not** advance when a PR gains or changes a
  closing reference to it, so the issues-since delta cannot see a new linked PR;
  the PRs-since query and the periodic full-load reconcile exist to cover that.
- Facts that a full reconcile re-fetches identically can never be healed by more
  refresh plumbing. When a value looks stale after a reconcile, suspect the
  query or the projection, not the cache.

## Classification (two-tier)

- Tier 1 (confident, automatic): `prd` label -> PRD; native PRD parent or
  `slice`/`ready-for-agent` label -> Slice.
- Tier 2 (heuristic, suggested): unlabeled issues scored by the planning-skill
  template headings (Problem Statement + User Stories -> PRD; What to build +
  Acceptance criteria / Blocked by / Parent -> Slice), surfaced with a
  "looks like a PRD/Slice — confirm?" badge and a confirm-and-add-label action.
- Tier 3: no match -> Unclassified, shown inline in "other open issues".
