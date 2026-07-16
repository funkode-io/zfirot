# Spec 0001 — Incremental board refresh + local board cache

**Status:** Draft
**Relates to:** ADR 0001 (read directly from GitHub, no backend), ADR 0002
(aggregates as state, replay deferred), ADR 0003 (agents live-discovered)
**Source:** Architecture review — board responsiveness & latency

## Problem

The board is a one-shot projection. The background poll (~60s), every mutation
(assign / delegate / confirm), **every navigation, and every project switch** all
run the same full path:

```
resolve_view → classify_board → load_issues → page 1..N (OPEN + CLOSED, 50/page)
                             → suggested_agents (extra round trip)
```

Consequences the user feels:

- **Switching between projects is slow** — each switch is a cold full load, and
  the previously-opened project's board is thrown away, so returning to it pays
  the full cost again.
- **A blank/spinner board on open** — opening or reopening a project shows a
  spinner until N+1 round trips finish; nothing is shown from what we already
  knew.
- **Mature repos are worst** — the whole closed-issue archive is paged every
  time, then discarded by `if raw.closed { continue }`.

The board module has **no seam for freshness** and **no retention** — the "how
fresh, how much to fetch, what do we already have" logic lives in the
presentation poll loop, not in the module that owns the board.

## Goal

The mental model, from the user's point of view:

> **A project is loaded in full exactly once, up front.** After that the
> interactive board only ever fetches the **latest deltas** — on every poll,
> restart, and project switch — painting instantly from the local cache. A
> separate **background reconcile** re-validates the whole project against
> GitHub on a slow cadence to self-heal anything a delta cannot see, and
> **Clean Cache** forces an immediate fresh load on demand.

Four deepenings deliver that:

1. **Retain-and-refresh** — a small `snapshot()` / `refresh()` interface with
   delta-merge and classification behind it, so a quiet poll costs **one small
   delta round trip**, not N+1.
2. **A local file cache, per project** — so opening, reopening, or **switching to
   a project paints its last-known board instantly** from disk, then applies
   only the delta on top. The full load is a **one-time seed** per project; the
   interactive path never blocks on a full load again. This mirrors the home
   screen's Projects cache.
3. **A cache-management control in the top bar** (new feature) — a small status
   indicator naming the current project's cache state, and a **Clean Cache**
   button that clears the cache for the current project or for all projects,
   forcing the next open to full-load and reseed.
4. **A background reconcile** (new feature) — on a slow cadence, silently run a
   full validation of the current project against GitHub and correct any drift
   the deltas could not catch (chiefly hard-deleted or transferred issues),
   without disturbing the instant-paint experience.

Target experience: **no blank board once a project has been seeded** — on launch
or switch, cached data paints immediately and the delta lands on top; the board
only full-loads again on an explicit Clean Cache.

## What is retained vs. what is dropped

| Data | Treatment |
|---|---|
| Open issues we've already seen | **Retained** in a per-project snapshot, **persisted to a local file** after a one-time full load; refreshed by delta, never re-paged wholesale (until Clean Cache). |
| Closed issues (the archive) | **Not fetched** on cold load (`states: [OPEN]`). Nothing to cache. |
| An issue that *closes* between refreshes | Appears in the delta (`[OPEN, CLOSED]` + `since`) as a **removal** — dropped from the retained set. |
| Assignable Agents | Retained on the snapshot; re-discovered on cold load (and always re-resolved live at action time, per ADR 0003). |

The "cache" is the **open-board snapshot, one per project, on disk**. Closed
issues are excluded by not fetching them in full, and flow through deltas only as
removals.

## Non-goals

- **No event sourcing / history.** `since` is real GitHub `updatedAt` data, not a
  fabricated timeline (ADR 0002 stays intact).
- **No change to the classification rules** beyond the one blocker-filtering fix
  below.
- **No caching of closed issues.** The archive is never fetched in full, so it is
  never stored.
- **No web/mobile persistence story.** The file cache is a desktop-only concern
  for v1, alongside the existing `FileProjectStore`.

## Design

Landed in order. Change 1 stands alone; the rest build on the retained snapshot
and its on-disk cache.

### Change 1 — Cold load fetches open issues only

`ISSUES_QUERY` moves from `states: [OPEN, CLOSED]` to `states: [OPEN]` for the
**cold load**. Pages then track *open work*, not total history.

Behaviour to verify (one table test): today a Slice whose native parent is a
**closed** PRD groups under that closed PRD's lane. A Done PRD is meant to be
hidden, so open-only drops that Slice to the "No PRD" lane — arguably fixing a
latent bug. Confirm this is the intended board.

### Change 2 — Retained snapshot + `snapshot()` / `refresh()` interface

Introduce a per-project snapshot the board module owns:

```rust
/// Everything needed to refresh the board without re-paging history.
/// Serde-serializable so it can be cached to a local file (Change 4).
pub struct BoardSnapshot {
    /// The open issues as last known (raw, pre-classification), keyed by number.
    open_issues: Vec<RawIssue>,
    /// The Assignable Agents from the last cold load.
    agents: Vec<AgentRef>,
    /// UTC instant captured at the *start* of the fetch that produced this
    /// snapshot — the `since` for the next delta.
    fetched_at: DateTime<Utc>,
}
```

`BoardService` gains two use-cases:

```rust
/// Cold load: full OPEN fetch + agent discovery + classify.
async fn load(&self, repo) -> AppResult<BoardView>;

/// Warm refresh: delta fetch since `previous.fetched_at`, merge, classify.
async fn refresh(&self, repo, previous: &BoardSnapshot) -> AppResult<BoardRefresh>;
```

where `BoardView { board: ClassifiedBoard, snapshot: BoardSnapshot }` and
`BoardRefresh` is `Changed(BoardView)` | `Unchanged` (mirroring
`ProjectsRefresh`, so an idle poll repaints nothing — no flicker).

Classification becomes a **pure** function over a raw issue set — it already is,
minus the fetch and the agent call — so both `load` and `refresh` share it.

### Change 3 — Delta port method

Add one method to the `GitHubPort` seam:

```rust
/// Issues (open AND closed) whose `updatedAt` is at or after `since`.
/// Closed ones are included so a close transition can be detected and removed.
async fn load_issues_since(&self, repo, since: DateTime<Utc>) -> AppResult<Vec<RawIssue>>;
```

Adapter query:

```graphql
issues(first: 50, after: $cursor,
       filterBy: { since: $since },
       orderBy: { field: UPDATED_AT, direction: DESC }) { … }
```

The delta is small (only recently-touched issues), so paging it is cheap even
though it includes closed transitions.

### Change 4 — Local file cache, per project (`BoardCachePort`)

A second seam, mirroring `ProjectStorePort`'s recent-projects cache
(`cached_projects` / `cache_projects`), but keyed per project:

```rust
#[async_trait]
pub trait BoardCachePort: Send + Sync {
    /// The last cached board snapshot for `repo`, or None on a cold cache.
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>>;
    /// Replace the cached snapshot for `repo`.
    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction;
}
```

File adapter: one entry per `owner/repo` under the app's data dir, alongside
`FileProjectStore`. In-memory fake for tests.

**Open / switch / reopen flow (stale-while-revalidate):**

1. Read `cached_board(repo)` — a **local** read, instant. If present, **paint the
   board immediately** from it (no spinner over content).
2. In the background, `refresh(repo, cached_snapshot)` — delta since
   `fetched_at`, merge, classify — then **update on top** and rewrite the cache.
3. **Cold cache** (first-ever open, or right after Clean Cache): fall back to
   `load` (open-only), paint, and seed the cache. This is the **only** full load
   a project ever does, and the only path that shows the loading spinner.

There is deliberately **no automatic re-seed** — no max-age, no periodic
reconcile. However stale a cache is, an open applies a delta on top of it; the
delta's `since` is the cached `fetched_at`, so a long-dormant project simply
catches up in one (larger, but still activity-bounded) delta on next open.

**Project switching** is the headline win: switching to any previously-seeded
project hits step 1, so its board paints instantly from disk while the delta
catches it up — instead of a cold full load every time.

### Merge semantics (pure, testable at the port seam)

```
retained: Vec<RawIssue>   // open issues from the cached/previous snapshot
delta:    Vec<RawIssue>   // open+closed issues updated since `fetched_at`

for d in delta:
    retained.remove(number == d.number)   // upsert: drop the stale copy
    if !d.closed { retained.push(d) }      // closed ⇒ just removed
# retained is now the merged open set
classify(retained, agents) -> ClassifiedBoard
```

Then the existing pure `resolve_unblocks` re-derives the reverse edge over the
merged set. Upsert-by-number is idempotent, so a `since` overlap that re-fetches
a few issues is harmless.

### The one sharp edge: stale blocker state

Deltas are keyed on `updatedAt`. Closing issue **B** bumps *B's* timestamp, not
that of issue **A** which is *blocked by* B. So after a delta, A could still
carry B in its `native_blockers` and wrongly stay **Blocked**.

**Fix (a deepening in its own right):** carry *all* native blockers (open and
closed) on `RawIssue`, and filter them to the board's currently-open set **inside
`classify_board`** — exactly as prose blockers are already filtered against
`open_numbers`. "Who is open" becomes one board-level fact decided in one place,
instead of being pre-baked per-issue in the adapter. When B leaves the retained
open set, A's blocker to B drops on the next re-derive. Locality: the blocked/not
decision concentrates in the classifier, and delta refresh becomes correct.

Linked-PR, parent, and assignee changes all raise timeline events on the Slice
itself, so they surface in the delta normally; blockers are the only cross-issue
hazard, and this fixes it.

### Change 5 — Cache usage indicator + Clean Cache control (top bar, new feature)

The app top bar gains a **global cache-usage indicator** showing the **total
disk space used by the board cache across all projects** (not just the open one).
It is a dropdown:

- **Collapsed** — the total size (e.g. "Cache 12.4 MB") plus a **clean icon**
  that clears **all** projects' caches.
- **Expanded** — a row **per cached project** (`owner/repo`), each showing that
  project's **space used** and its own **clean icon** that clears **only that
  project**.

Clearing behaviour:

- **Clean icon on the global indicator** → `clear_all`: drops every cached
  snapshot. Each project full-loads and reseeds on its next open.
- **Clean icon on a project row** → `clear_board(repo)`: drops just that
  project's snapshot; it reseeds on its next open. It need not be the currently
  open project.

The list is global cache state, so the indicator reflects every project the user
has ever seeded, independent of which board is on screen. Clearing is the user's
immediate escape hatch; the background reconcile (Change 6) is the automatic one.

### Deletes / transfers: healed by background reconcile

A hard-deleted or transferred-out issue never appears in a delta (nothing bumps a
timestamp we can see), so deltas alone would let it linger. Two mechanisms clear
it: the **background reconcile** (Change 6) heals it automatically on the next
reconcile pass, and **Clean Cache** (Change 5) heals it immediately on demand.
Every other change — close, reopen, edit, new issue, assignment, linked PR —
rides the delta normally and needs neither.

**Worked example.** Slice #42 sits in the WIP column of a seeded project. A
maintainer **deletes** #42 on GitHub (or transfers it to another repo). Deleting
an issue raises no `updatedAt` event we can observe — the issue is simply gone —
so the next delta (`load_issues_since`) returns nothing about #42, and the merge
never removes it, leaving a ghost card. On its next pass the **background
reconcile** full-loads the project, sees #42 is no longer among the open issues,
and drops it — the card disappears with no user action. A user who wants it gone
now instead presses **Clean Cache (this project)**, which reseeds immediately.
Contrast a **close**: closing #42 bumps its `updatedAt`, so it arrives in the
delta as `closed`, the merge removes it, and the card disappears on the next
poll — no reconcile or manual step needed.

### Change 6 — Background reconcile (full validation on a slow cadence, new feature)

Deltas keep the board *fast*; reconcile keeps it *correct*. On a slow cadence
(much longer than `PollInterval` — see open questions) the app runs a full
validation of the **currently-open project** in the background and reconciles the
cache with GitHub, the source of truth.

**Mechanism — reuse, don't add.** A reconcile is just the existing cold `load`
(open-only, full) run in the background, its result compared against the cached
snapshot: if they differ, swap the cache and repaint; if they match, do nothing
(`Unchanged`, no flicker). No new port method — it reuses `GitHubPort::load_issues`
and `BoardCachePort`. Because the full load is authoritative, the reconcile
result simply *becomes* the new snapshot: deletes/transfers vanish, any drifted
derived state is corrected, and the Assignable-Agent set is refreshed for free
(healing the agent-staleness noted below).

**Cadence & scope.** A `ReconcileInterval` domain value object (a clamped value
object like `PollInterval`) governs the cadence. v1 reconciles the
currently-open project only; extending the sweep to every Tracked repo in the
background is a scope decision deferred to the open questions (it multiplies
round trips and rate-limit pressure).

**Non-blocking, silent.** Reconcile never shows a spinner over the board and
never blocks interaction — it swaps the snapshot underneath only when it finds a
difference, exactly like the delta refresh's `Changed` path. Its effect is
visible only through the board updating and the cache-usage figure (Change 5)
adjusting as issues are added or dropped.

### Agent discovery

`refresh` reuses `previous.agents` rather than re-running `suggested_agents`, so
agent discovery leaves the per-poll hot path. The Assignable-Agent set is
discovered on the one-time full load and refreshed by the **background
reconcile** (Change 6) and by Clean Cache; the delegate action re-resolves the
chosen Agent's node ID live at action time regardless (ADR 0003), so a stale set
only affects whether the delegate control is offered, not correctness. No
read-model contract change (ADR 0003 holds).

## Interfaces changed

- `GitHubPort`: **+** `load_issues_since`; `load_issues` narrows to OPEN (and is
  reused as-is by the background reconcile's full load).
- **+** `BoardCachePort`: `cached_board` / `cache_board` / `clear_board(repo)` /
  `clear_all`, plus `cache_usage` returning per-project disk usage
  (`Vec<{ repo, bytes }>`, total = sum), per project. File adapter + in-memory
  fake.
- `BoardService`: **+** `load`, `refresh`, and `reconcile` (background full-load
  validate-and-swap); `classify_board` splits into fetch + pure `classify`.
- **+** `ReconcileInterval` domain value object (clamped, like `PollInterval`).
- Presentation top bar: **+** a global cache-usage indicator (total size) with a
  per-project breakdown dropdown, and clean icons at both the global (`clear_all`)
  and per-project (`clear_board`) levels.
- `RawIssue`: `native_blockers` carries open **and** closed (state moves to the
  classifier). Made serde-serializable (it lands in the cached snapshot).
- `BoardService`: **+** `load`, `refresh`; `classify_board` splits into fetch +
  pure `classify`.
- Application: **+** `BoardSnapshot` (serde), `BoardView`, `BoardRefresh`.
- Presentation: open/switch/reopen paints the cached snapshot, then `refresh`es
  on top; the retained snapshot lives in a signal; the poll loop refreshes it,
  and a slower background loop reconciles it.

## Testing

Two seams; both already have precedent in the codebase.

- **`GitHubPort` fakes** — `load_issues_since` returns canned deltas; no network.
  Covers refresh, merge, and the delta path (prior art: existing
  `crates/infrastructure/tests/` fakes).
- **`BoardCachePort` fake** — in-memory cache; covers the stale-while-revalidate
  open/switch flow, cold-cache (first-open / post-Clean-Cache) full-load
  fallback, `cache_usage` reporting per-project size and total, and that
  `clear_board` / `clear_all` force the next open to reseed (prior art: the
  projects-cache fake behind `RecentProjectsService`).
- **Pure `classify`** — table-tested: the blocker-filtering fix, open-only
  cold-load behaviour, closed-parent-PRD lane (prior art: `resolve_unblocks`,
  `SliceState` derivation).
- **Pure merge** — table-tested: upsert, close-removal, new-issue add,
  `since`-overlap idempotence, and the stale-blocker regression.
- **Refresh outcome** — `Unchanged` on an empty delta (no repaint); `Changed`
  otherwise.

Good tests here assert external behaviour (what board a given cache + delta
produces), never internal call counts.

## ADR alignment

- **0001** — GitHub stays the source of truth. The board cache is a
  **revalidated read-model cache**, not owned state — the same standing the
  Projects list cache already has (`cached_projects` / `cache_projects`). No
  backend, no new owned aggregate. No ADR amendment required.
- **0002** — `since` is real `updatedAt`, not a fabricated event stream.
- **0003** — agents still live-resolved at action time; retention is only a
  per-refresh optimisation.

## Suggested slicing

1. **Open-only cold load** (Change 1) — one-line query change + one table test.
   Immediate page cut, ships alone.
2. **Blocker filtering moves to the classifier** — prerequisite correctness fix
   for delta refresh; valuable on its own.
3. **Retained snapshot + `snapshot()`/`refresh()`** (Change 2, in-memory) — the
   deepening; poll reuses the snapshot but still full-fetches until 4 lands.
4. **Delta port method + merge** (Change 3) — quiet poll = one small round trip.
5. **`BoardCachePort` + stale-while-revalidate paint** (Change 4) — the
   instant-paint-on-switch win, seeded once then deltas forever; depends on 2–4
   for the snapshot shape.
6. **Cache-usage indicator + Clean Cache control** (Change 5) — top-bar global
   total, per-project breakdown dropdown, and clean icons at both levels
   (`clear_all` / `clear_board`); the user's immediate reseed trigger.
7. **Background reconcile** (Change 6) — `reconcile` use-case + `ReconcileInterval`
   + the slow background loop; the automatic self-heal for deletes/transfers and
   drift. Depends on 5 for the cache and status surface.

## Open questions

1. **Reconcile cadence** — the `ReconcileInterval` default (e.g. every 15–30 min,
   or once per session)? Fixed for v1 or a setting?
2. **Reconcile scope** — current project only (proposed for v1), or sweep every
   Tracked repo in the background (more round trips, more rate-limit pressure)?
3. **Cache location & format** — one file per repo vs. a single map file;
   alongside `FileProjectStore` in the app data dir. JSON via serde?
4. **Cache eviction** — cap the number of cached projects, or leave unbounded
   for v1 (Clean Cache (all) is the manual sweep)?
5. **Cache-usage display** — size units/rounding ("12.4 MB"), and whether the
   indicator appears only on the board top bar or also on the home screen; how a
   cache-usage read gets its per-project byte counts (file sizes vs. serialized
   length).
6. **`since` boundary** — capture at request *start* (proposed) and accept the
   small overlap, confirmed?
7. **Surface discrepancies?** — does reconcile silently self-heal (proposed), or
   also hint that it corrected something (e.g. a brief "updated N issues" note)?

_Resolved by this revision:_ deltas stay the fast interactive path (no max-age
gate on open); correctness drift is healed by the slow background reconcile and,
immediately, by Clean Cache.
