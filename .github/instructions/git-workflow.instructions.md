---
description: Branching, commits, PRs, and syncing with upstream
---

# Git Workflow

Load this when creating branches, committing, or opening PRs.

- The planning flow that feeds this repo:
  `grill-with-docs` (design) -> `to-prd` (publish PRD issue) -> `to-issues`
  (publish Slice issues) -> this app visualises the result.
- Issues labelled `slice` + `ready-for-agent` are grabbable units of work. Each
  is a thin vertical tracer bullet through every layer.
- `## Blocked by` in an issue body lists prerequisite Slices. Do not start a
  Slice whose blockers are still open.
- Branch naming: `feat/<short-slug>-<issue#>` off `upstream/main`.
- PR titles MUST be Conventional Commits (squash merge derives the commit msg).
- Sync with main via a MERGE commit (`git merge upstream/main`), never rebase.
- NEVER force-push a branch with an open PR; address review with new commits.
- Remotes: `upstream` = funkode-io/zfirot, `origin` = carlos-verdes/zfirot.

## During code review

- NEVER force-push. Push only NEW commits on top of the branch; never amend,
  rebase, or otherwise rewrite already-pushed history while a PR is open — it
  breaks incremental review.
- Address each piece of feedback with a fresh commit.
- PRs are merged with **squash merge**, so the individual review commits are
  flattened into one on merge — there is no need to tidy history by force-pushing.
- The PR title MUST be a valid Conventional Commit (e.g.
  `feat(presentation): …`, `fix(domain): …`), because the squash-merge commit
  message is derived from it.
