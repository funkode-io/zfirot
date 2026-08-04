---
status: accepted
---

# Refresh by authoritative sweeps and targeted hydration

The board asked GitHub *"what changed since my last poll?"* and trusted the
answer. GitHub does not answer that question honestly: an issue's `updatedAt`
does not move when a PR opens with a closing reference to it, and a PR merging
leaves no trace at all on the issues it unblocks. Every recent fix is the same
bug wearing a different hat — the PR-since patch query (#150), open-only Linked
PRs (#161), the node-limit blowout that stopped that patch from ever running
(#168) — and the user-visible cost is a board that shows a merged PR as ongoing
while the six Slices it unblocks sit in Blocked (#179). We stop consuming a
change feed we cannot verify and instead run two **sweeps** that ask what
*exists* — every **open PR** with its full status, and every **open issue
number** with its `updatedAt` — treat each sweep as **authoritative** (anything
absent from it is gone), and **hydrate** full issue detail **by number** only for
what a sweep reports as new or changed. Drift then has nowhere to hide: there is
no "GitHub didn't tell us" path left, because we stopped asking GitHub to tell
us.

Sweeps are affordable precisely because they are shallow. Measured on
`majidalfuttaim/dxp-data-loom` (67 open issues, 14 open PRs) with
`rateLimit { cost nodeCount }`, against a 5,000 points/hour budget:

| call | points | bytes |
|---|---|---|
| PR sweep — every open PR, full spine + Decorations + `closingIssuesReferences` | 3 | 9.5 KB |
| issue sweep — `{ number updatedAt }` for every open issue | 1 | ~5 KB |
| hydrate 5 changed issues by number (aliases, one round trip) | 2 | — |
| full hydrate of all 67 open issues (the safety net) | 4 | 300 KB |
| _for comparison:_ today's delta poll (issues-since + PRs-since) | **68** | — |
| _for comparison:_ today's full reconcile | **46** | 300 KB |

Three clocks follow from the two things the user distinguishes. **Hot (15s):** the
PR sweep — every fact on the critical path (new review comments, CI, conflicts,
approvals, a PR appearing, a PR merging) lives on the PR, and the sweep is
authoritative over the open-PR set, so it discovers and prunes without depending
on cached state. **Structural (60s):** the issue sweep plus hydration of what
moved — grooming output is *expected*, so it may lag. **Safety net (300s):** a
full hydrate of every open issue, because GitHub demonstrably fails to bump
`updatedAt` for some relationship edits and the design assumes it lies rather
than trusting it.

One event breaks the clock discipline deliberately. A change to the **open-PR
set** immediately **cascades** into a hydrate of exactly the issues those PRs
close, without waiting for the structural clock. That is the reported failure
path end to end: PR merges → it leaves the open-PR set within 15s → its closing
issues are hydrated → they read closed → and their dependents flip Blocked →
Ready in the same repaint, with **no further fetch**, because blocker openness is
derived from the snapshot rather than fetched.

## Considered options

- **Keep the delta design and just budget the node count** (what #169 did).
  Rejected as the whole problem: the corrected queries cost a measured **4,632
  points/hour for one idle board** — 93% of the token's budget, so a second board
  or a second running instance gets throttled, and throttling is staleness with
  extra steps. It also leaves the change feed's blind spots intact, which is the
  bug class that has now bitten three times.
- **Alias-hydrate only the Linked PRs already in the cache** (1 point instead of
  3). Rejected: it is blind to a PR *appearing* — a colleague opening a PR on
  your Slice would wait for the structural clock. That is the same class of
  surprise as a comment you did not know about, and the 2-point saving does not
  buy it.
- **Sweep everything deeply on every poll** (no hydration step). Rejected: cost
  and bandwidth scale with repo size rather than with activity (300 KB per poll
  here, mostly issue bodies that change rarely), and a 1,000-issue repo would
  push a 60s cadence back over the budget.
- **Webhooks, or any push channel.** Rejected by ADR 0001 — it needs a backend to
  receive them. Sweeps are the honest read-only substitute.
- **A longer safety-net cadence instead of sweeps** (trust `updatedAt`, reconcile
  rarely). Rejected: it is today's design, and the reported bug is exactly what it
  produces.

## Consequences

- **`RawIssue` loses `linked_prs`, and the issue queries carry no PR data at
  all.** The PR→issue edge is read from the PR side (`closingIssuesReferences`),
  which is the same edge from the other end. The four-deep nesting that produced
  #168 ceases to exist rather than being budgeted, and the issue side gets ~10×
  cheaper (46 → 4 points for a full load) as a side effect.
- **"Linked PR means an *open* PR" (#161) becomes true by construction**, since
  the sweep only ever lists open PRs, instead of being a client-side filter over
  a connection that also returns merged ones.
- **`BoardSnapshot` splits into two independently-owned collections** — issues
  and linked PRs — joined by the pure `classify` at paint time. One writer per
  fact: the hot loop can never be reverted by a slower structural write carrying
  an older PR list. This is a serde format change, so cached snapshots from older
  versions are discarded rather than mis-read.
- **`ISSUES_SINCE_QUERY`, `PULL_REQUESTS_SINCE_QUERY`, the `filterBy: { since }`
  watermark, and `reconcile_cached` are deleted.** Deletes, transfers and manual
  closes heal on the 60s issue sweep rather than on a 5-minute full reconcile, so
  the mechanism that spec 0001 introduced for them is superseded; that spec's
  cache and instant-paint decisions stand unchanged.
- **Cost scales with open PRs and with change, not with repo size.** Steady state
  is ~950 points/hour worst case (~19% of budget), leaving room for several
  boards. A repo with more than 100 open PRs pages the hot sweep at 3 points per
  page.
- **The node-budget guard test (#169) stays and covers every query**, including
  the alias-hydrate chunks (capped at 50 aliases). It is the only offline defence
  against a selection set growing back into a limit, since parse tests replay
  recorded bodies and cannot see a query GitHub would reject.
- **A hydrate aliases issues by number, and GitHub answers a missing number with
  a field-level error beside the data.** That is how deletion and transfer are
  detected, and it is why the parsers must keep tolerating partial errors.
- **`mergeable` is computed asynchronously by GitHub** and reads `UNKNOWN` for a
  moment after a push. At a 15s cadence it settles quickly, and ADR 0004's rule
  (decorate only true `CONFLICTING`) means the transient state shows nothing
  rather than a false conflict.
- **The hot loop keeps polling while the window is unfocused.** Pausing it would
  make the first glance after switching back the stalest one — the exact moment
  the user is looking. Revisit only if the budget becomes tight with many boards.
