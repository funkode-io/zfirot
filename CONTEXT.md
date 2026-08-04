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
PRD. Having a **Parent** that resolves to a PRD is enough to make an issue a
Slice, whatever its labels — a Slice is not required to be workable, and the
not-yet-workable ones matter most (they are the plan's unfinished edges).
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
A Slice with an open Pull Request linked to it (via the PR's closing reference). Its finer substate is its Best PR's PR status, decorated with any of that PR's Decorations. An open PR is live evidence of work and outranks any Blocked reason — except an unstacked PR on a **Dependency**-blocked Slice, where GitHub's own branch topology says the work started out of order.
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

## Pull request status

**PR status**:
The review-lifecycle stage of an open Linked PR, a single ordered axis: **Draft** (author still working) -> **Awaiting review** (waiting on a reviewer) -> **Changes requested** (reviewer bounced it back) -> **Approved**. Purely the review dimension, derived from GitHub's draft flag and `reviewDecision`; merge-health signals are Decorations that ride on top, not stages of this axis.
_Avoid_: PR state, merge state, review state

**Best PR**:
When a Slice has more than one open Linked PR, the one with the highest PR status — it represents the real work and drives the Slice's WIP substate. The others are redundant and should be closed; seeing a lower-status badge (e.g. a Draft next to an Approved) is what tells you which to close.
_Avoid_: primary PR, main PR

**Decoration**:
An orthogonal attention marker on a Linked PR, shown as a small hover icon that fires independently of (and on top of) its PR status. Three exist: **Conflicts**, **Unresolved comments**, **CI failing**.
_Avoid_: flag, badge (a badge is the `pr #n @u` element itself)

**Conflicts**:
A Linked PR that needs a manual conflict merge — GitHub `mergeable = CONFLICTING`. Merely being behind the base branch (auto-updatable, `BEHIND`) is not flagged.
_Avoid_: out of date, behind

**Unresolved comments**:
The count of a Linked PR's review threads still open (`isResolved = false`). Non-blocking — you can merge without addressing them — but surfaced so you know they exist (e.g. an agent's follow-up notes).
_Avoid_: open comments, review comments

**CI failing**:
A Linked PR whose check rollup is failing or errored (`statusCheckRollup = FAILURE | ERROR`). Pending checks are transient and not flagged.
_Avoid_: build broken, checks red

**Stacked PR**:
A Linked PR branched on another open PR instead of the default branch — its base
branch is another open PR's head branch. It marks work deliberately begun on top
of unfinished work, so a Slice blocked by a **Dependency** whose PR is stacked is
being worked *correctly*, not out of order. The marker names the PR it sits on.
_Avoid_: chained PR, dependent PR

**Ready to merge**:
Not a stored state — the plain-English reading of an **Approved** PR carrying no red Decorations (no Conflicts, CI not failing). Unresolved comments do not disqualify it, since they do not block a merge.
_Avoid_: mergeable (that is one GitHub field, not this synthesis)

**Blocked**:
A Slice that is **not workable** — nobody, human or Agent, can pick it up right
now. Every Blocked Slice carries a **Blocked reason** saying why. An open
dependency is only the most common reason; a Slice can be Blocked with no
blocker badges at all.
_Avoid_: waiting, stuck, deferred (that is one reason, not the state)

**Blocked reason**:
Why a Blocked Slice cannot be picked up, shown as a chip on its card. Five
exist: **Dependency** (an open "blocked by" issue), **Needs triage** (a
maintainer must decide keep / refine / close), **Deferred** (deliberately put
off), **External blocker** (waiting on something that is not a GitHub issue),
and **Labelled blocked** (a human said so and gave no other evidence). Only
**Dependency** is self-updating — it disappears by itself when the blocking issue
closes. The other four are human-authored and sticky: they persist until someone
edits the issue, so they can go stale and are outranked by live evidence of work.
When several apply at once they rank by how much they say about *what must
change* — Dependency > External blocker > Needs triage > Deferred > Labelled
blocked — and the top one is the Slice's reason for display and ordering.
_Avoid_: block type, blocked kind

**External blocker**:
A prose "blocked by" entry that names no GitHub issue — e.g. "a prod values file
existing for this app". Real enough to stop the work, but invisible to the
dependency graph, so it can never be an edge and never closes itself.
_Avoid_: soft blocker, non-issue dependency

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

**Suggested PRD**:
An issue that reads like a PRD by its headings alone, with no `prd` label to say
so. It heads a Lane like any PRD, marked as unconfirmed, and its Slices are found
the same way — so a plan is visible before anyone has labelled it.
_Avoid_: maybe-PRD, candidate

**Confirm**:
Accept a suggestion by adding the label it lacks (`prd`), turning a guess into a
fact everyone shares. The app only ever records decisions on GitHub — it keeps no
local memory of what the user has judged.
_Avoid_: approve, accept

**Veto** (label: `not-a-prd`):
Reject a suggestion outright, so the app stops guessing about that issue. The
negative counterpart of **Confirm**, and deliberately also a GitHub label: a
rejection is worth as much to a teammate as an acceptance, and holding it locally
would be private, cache-fragile state that fights whatever GitHub later says.
_Avoid_: dismiss, ignore, hide

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
chain reads as `first → next → last`. Because the horizontal axis *is* dependency
order, a Slice with no dependency edge at all that is also not workable has no
honest position on it, and sits in a detached **rail** on the right rather than
among the roots — the left edge must only ever mean "start here". A single global
toggle switches the whole board between the two and the choice is remembered
across launches. Both render the same Slices with the same states; only the
arrangement differs.
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

## Freshness

**Sweep**:
An authoritative listing of everything of one kind that currently exists on
GitHub — every **open PR** with its full status (every 15s), and every **open
issue number** with its `updatedAt` (every 60s). A sweep is not a change feed and
asks no "what's new?" question: its answer *is* the truth, so anything missing
from it no longer exists and is dropped. Sweeps are affordable because they are
shallow — they carry no nested detail — and their cost tracks the number of open
PRs and issues, not the size of the repo.
_Avoid_: poll, delta, sync

**Hydrate**:
Fetch the full detail of specific issues **by number** — body, labels, assignee,
Parent, blockers — for the ones a **Sweep** reports as new or changed. The
expensive part of a refresh happens only for things known to have moved, so an
idle board pays almost nothing.
_Avoid_: fetch, load, enrich

**Cascade**:
A **Hydrate** triggered by a **Sweep** result instead of by a clock. Any change
to the open-PR set immediately hydrates the issues those PRs close: a PR merges,
its Slice reads closed, and every Slice it was blocking becomes **Ready** in the
same repaint — no extra fetch, because blocker openness is derived from what the
board already holds.
_Avoid_: chain, trigger, ripple

**Freshness contract**:
The promised maximum staleness per class of fact, and the thing to hold the app
to: **15s** for anything on a PR (new comments, CI, conflicts, approvals, a PR
appearing, a PR merging and its dependents unblocking), **60s** for issue
structure (a new PRD or Slice from grooming, a retitle, a relabel, a manual
close), **5 min** for what GitHub silently fails to timestamp (deleted or
transferred issues, edited relationships), and **instant** for opening the app or
switching project, which paints from cache first. PR facts are the critical path
because they are the ones nobody tells you about; grooming output can lag,
because you already know it is coming.
_Avoid_: refresh rate, TTL, SLA

## Identity and authentication

**Viewer**:
The authenticated GitHub user the stored token acts as. The app reads the
Viewer's `login`, `name`, and `avatarUrl` (GitHub's `viewer`) to show an account
menu; the token itself carries no profile.
_Avoid_: current user, account, me

**Rotate** (UI label: "Change token"):
Replace the stored token with a freshly-granted one for the **same Viewer**,
atomically — the old token stays valid until the new one is validated and saved,
and all account-scoped local state is preserved. The action used when the app's
required grants grow (a new feature needs a new permission).
_Avoid_: renew, refresh (refresh is the board action)

**Sign out**:
Remove the token **and** all account-scoped local state (tracked repos,
last-opened project, cached recent projects, cached boards), resetting the app to
its first-run, no-token state. UI preferences (theme, view mode) survive because
they are not tied to the Viewer.
_Avoid_: log out, disconnect

**Invalid token**:
A stored token GitHub rejects as unusable — revoked, expired, or malformed
(Unauthorized / 401). It is useless, so the app clears it and routes to the
paste-token screen.
_Avoid_: bad token, expired (only one of the causes)

**Under-scoped token**:
A token that is valid but missing a grant an operation needs (Forbidden / 403).
It still works elsewhere, so the app **never clears it**; instead it offers to
Rotate, carrying the specific missing permission so the required-permissions list
can highlight the exact grant to add.
_Avoid_: insufficient token, unauthorized (that is the Invalid case)
