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

## Avoiding silent merge-loss across parallel PRs

Two PRs that are each green in isolation can still corrupt `main` when merged in
sequence, because the local `pre-push` hook only ever sees one branch. This has
happened twice:

- **Build break (#81 + #89):** #81 added a `RawIssue` field; #89's test built
  `RawIssue` without it. Merged together, `main` failed to compile.
- **Silent feature-loss (#110 + #111):** both rewrote the same `BoardShell`.
  #111 was branched off `main` *before* #110 merged, so merging #111 reverted
  #110's theme switcher — and it still **compiled**, so nothing flagged it.

The second class is the dangerous one: a merge can delete a shipped feature
without any conflict marker or CI failure, because the reverting side simply
never references the deleted code. Guard against it:

- **Merge only from a branch that is up to date with `main`.** Before merging,
  `git merge upstream/main` into the PR branch and let CI re-run on the combined
  result. Branch protection enforces this ("require branches to be up to date").
  This turns a silent revert into a visible conflict you must resolve.
- **When a sibling PR merges, immediately sync every other open PR** with
  `main`. Do not let an open branch drift across a sibling merge — the longer it
  drifts, the more likely its stale copy of a shared file wins.
- **Do not parallelise PRs that edit the same hot file** (e.g. `app.rs` /
  `BoardShell`). If two frontier tickets both touch one component, **stack**
  them (one based on the other's branch) instead of opening both off `main`.
- **After merging a PR that touches a shared file, verify no sibling feature
  was reverted** — grep `main` for the sibling's key symbols (e.g. a type,
  component, or port method it introduced) before moving on.
- Resolving a conflict means **keeping both features**, never taking one whole
  side blindly. A clean `cargo build` is not proof the merge was correct.

## Before pushing

- Run `make hooks` once after cloning to install the version-controlled git
  hooks (`.githooks/`, wired via `core.hooksPath`).
- The `pre-push` hook runs `make check` (`cargo fmt --all --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`) and
  blocks the push if any step fails. Fix the failure rather than bypassing.
- Bypass only in a genuine emergency with `git push --no-verify`.

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

## When creating a pull request

- Title MUST be Conventional Commits (the squash merge derives the commit
  message from it), e.g. `feat(presentation): render linked PRs`,
  `fix(domain): guard nil parent`.
- Body must reference the GitHub issue it addresses so reviewers can follow
  context and so GitHub closes the issue on merge. Use one of:
  - **Single issue / single commit:** close the ticket with `Closes #NN` (also,
    mention that same number in the title, e.g. `… (#79)`).
  - **Multiple issues** (merged work): list them as bullet points and close
    each explicitly: `- Closes #NN`, `- Closes #MM`. Do not use a blanket
    `Closes #NN-MM` if it spans unrelated tickets.
- Include:
  1. **What changed:** briefly map each layer touched.
  2. **Acceptance criteria checklist** (if the issue had one) with `[x]` marks,
     so reviewers can verify completeness at a glance.
