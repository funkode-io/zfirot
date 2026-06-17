# Zfirot

A multi-platform (web, desktop, mobile) dashboard that visualises the state of
agent-driven coding work tracked on GitHub: which PRDs are open, which slices are
ready to be picked up, in progress, or blocked.

## Language

**PRD**:
A product requirements document describing a feature. Represented as a GitHub
Issue carrying the `prd` label.
_Avoid_: spec, epic, story

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

**Blocked**:
A Slice with at least one open "blocked by" dependency.
_Avoid_: waiting, stuck

**Done**:
A closed Slice or PRD. Hidden from the active board.
_Avoid_: complete, finished, merged

**Agent**:
A non-human worker that can be given a Ready Slice to work on. In v1 the only
Agent is GitHub's hosted Copilot coding agent, delegated to by assigning its bot
account to the Slice's issue. Self-hosted Agents (containers the app launches)
are a later phase. Delegating to an Agent is one way to pick up a Slice; the goal
is to parallelise work across every available Agent.
_Avoid_: bot, worker, copilot (the specific provider, not the role)

**Assignable Agent**:
An Agent that can currently be given a Slice on a given repo, discovered live per
board. The board carries the full set of them (zero or more); in v1 the set is
either empty or just GitHub's Copilot, but later it also includes any local
Agents. When a Slice is delegated the user picks which Assignable Agent does the
work. Carried on the board read model as current state, never persisted.
_Avoid_: available bot, copilot id, the agent (there may be several)

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
