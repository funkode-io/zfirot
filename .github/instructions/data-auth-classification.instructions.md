---
description: GitHub GraphQL data source, PAT auth, freshness, and issue classification
applyTo: "crates/{infrastructure,application}/**/*.rs"
---

# Data, Auth & Classification

## Data & auth

- Data source: GitHub **GraphQL** only. The board is read as two collections:
  one query returns a project's issues with parent, dependencies, assignees and
  labels (and **no** Pull Request data), and an open-PR **Sweep** returns every
  open PR with its status, Decorations, and the issue numbers it closes. The
  pure `classify` joins them by issue number. Mutations: assign self
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
- An issue's `updatedAt` does **not** advance when a PR gains or changes a
  closing reference to it, so an issues-since delta can never see a new Linked
  PR. That is why the open-PR sweep runs on **every** refresh path (cold load,
  cached open, poll, reconcile) rather than being derived from the issue delta.
- Facts that a full reconcile re-fetches identically can never be healed by more
  refresh plumbing. When a value looks stale after a reconcile, suspect the
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
