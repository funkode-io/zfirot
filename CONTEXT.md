# Zfirot

A multi-platform (web, desktop, mobile) dashboard that visualises the state of
agent-driven coding work tracked on GitHub: which PRDs are open, which slices are
ready to be picked up, in progress, or blocked.

## Language

**PRD** (alias: **Spec**):
A product requirements document describing a feature. Represented as a GitHub
Issue carrying the `prd` (or `spec`) label. **PRD** is the canonical term; the
`to-spec` skill (which replaced `to-prd`) calls the same artifact a **Spec** —
treat the two as synonyms. An issue is classified as a PRD if it carries
*either* label, so a lane is still captured if the tooling ever emits `spec`.
_Avoid_: epic, story ("spec" is now an accepted alias, not forbidden)

**Slice**:
A thin vertical tracer-bullet unit of work that cuts end-to-end through every
layer and is independently grabbable. Represented as a GitHub Issue, child of a
PRD.
_Avoid_: task, ticket, subtask

**Parent**:
The PRD a Slice belongs to. Read from GitHub's native sub-issue (parent–child)
relationship when present, otherwise parsed from the issue body's `## Parent`
section (the current skill output).
_Avoid_: epic link, owner

**Blocked by**:
A dependency from one Slice to another that must close first. Read from GitHub's
native issue dependency relationship when present, otherwise parsed from the
issue body's `## Blocked by` section (the current skill output).
_Avoid_: depends on, waiting on

**Ready**:
A Slice with all blockers closed, no open linked PR, and no assignee — free for
an agent to pick up.
_Avoid_: open, available, todo

**WIP**:
A Slice with an open Pull Request linked to it (via the PR's closing reference).
_Avoid_: in progress, active, doing

**Linked PR**:
An open Pull Request that closes a Slice's issue (GitHub's closing reference).
A Slice's open linked PRs share a dedicated row on its card, one `pr #n @u`
badge each, where `n` is the PR number and `u` is the PR's **author** (for
delegated work, the Agent's bot account) — on any card that has one, regardless
of column. Closed PRs are not shown, so replacing one PR with another leaves
only the still-open one. Clicking a badge opens that PR; hovering shows its
title. When the Slice is **Blocked**, each PR badge carries a warning marker,
since the PR is being worked on while the Slice still has an open dependency
that should land first.
_Avoid_: closing PR, the PR (a Slice may have more than one open)

**Blocked**:
A Slice with at least one open "blocked by" dependency.
_Avoid_: waiting, stuck

**Done**:
A closed Slice or PRD. Hidden from the active board.
_Avoid_: complete, finished, merged

**Agent**:
A non-human worker that can be given a Ready Slice to work on (in v1, GitHub's
hosted Copilot coding agent). Hand-off to an Agent happens **outside the app**
via a dedicated PR-creation skill that opens a PR and comments to the agent; the
app itself no longer assigns Agents (the GitHub delegate mutation proved
unreliable). The goal is still to parallelise work across every available Agent.
_Avoid_: bot, worker, copilot (the specific provider, not the role)

**Assignable Agent** _(removed)_:
Previously the live-discovered set of Agents the app carried on the board read
model and let the user delegate a Slice to. Removed together with in-app Agent
assignment; kept here so older commits and the superseded ADR 0003 still read
coherently.

**Unclassified issue**:
An open GitHub Issue the app cannot confidently map to a PRD or Slice. Surfaced
on the dashboard as "other open issues" with no further action.
_Avoid_: misc, unknown, orphan

**Lane**:
A horizontal swimlane on the board grouping every Slice that belongs to one PRD.
Each lane has a header linking to its PRD Issue and contains the Ready / WIP /
Blocked columns for that PRD's Slices. Slices with no parent PRD collect in a
trailing "No PRD" lane.
_Avoid_: row, group, section

**Graph view** (vs **Columns view**):
Two ways to render a Lane. **Columns view** (the default) lays a PRD's Slices
out in Ready / WIP / Blocked columns. **Graph view** draws them as a
left-to-right **Blocked by** graph — dependency roots on the left, so a stacked
chain reads as `first → next → last`. A single global toggle switches the whole
board between the two and the choice is remembered across launches. Both render
the same Slices with the same states; only the arrangement differs.
_Avoid_: DAG view, tree view, pipeline, flow

**Tracked repo**:
An `owner/repo` the user summoned by name on the home screen — rather than
having it discovered — and that the token could open. The act of opening it
successfully is what tracks it; a repo the token cannot reach is never tracked.
Persisted locally and shown on home alongside discovered Projects, surviving
restarts. The set of tracked repos is the one piece of domain state the app owns
locally.
_Avoid_: project, watch, source

**Project**:
A repo as presented on the home screen. Home lists the most recently active
projects (by last push); a "Show more" reveals the rest of the user's repos.
_Avoid_: workspace, board

**Aggregate**:
A locally-owned unit of state defined as a serde state struct plus commands,
deliberately not yet wired to the replay framework. `Prd` and `Slice` are NOT
aggregates — they are read models projected from GitHub.
_Avoid_: entity, model

**Last opened project**:
The project the user was viewing when the app last closed, persisted on the
local device only so the app reopens there on next launch.
_Avoid_: recent, history
