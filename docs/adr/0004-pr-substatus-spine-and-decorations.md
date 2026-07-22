---
status: accepted
---

# PR sub-status: a review-only spine plus orthogonal decorations

A WIP Slice's linked PR carries far more state than "open" — draft, review
decision, CI, mergeability, unresolved comments. A follower needs to know *whose
court the ball is in* and *what still blocks the merge*, at a glance. We model
this as a single ordered **review-only spine** — `Draft → Awaiting review →
Changes requested → Approved` — with merge-health signals layered on top as
independent **Decorations** (**Conflicts**, **Unresolved comments**, **CI
failing**), rather than as one flat combined status enum. **"Ready to merge" is
not a stored state**: it is the derived reading of `Approved` with no red
decorations. When a Slice has more than one open PR, the highest-status **Best
PR** drives the Slice headline and the lower one is visibly the one to close.
This is **additive** to `SliceState` (new fields on `LinkedPrRef` + a `Best PR`
derivation); the four board states and their precedence are untouched.

## Considered options

- **Flat combined status enum** (e.g. `Draft | AwaitingReview | ChangesRequested
  | CiFailing | Conflicts | ReadyToMerge`). Rejected: the axes are genuinely
  orthogonal in GitHub — a PR can be `Approved` *and* conflicting *and* have
  unresolved comments — so a single enum either loses information or explodes
  combinatorially (`ApprovedButConflicting`, `ApprovedWithComments`, …). The
  spine + decorations split composes the exact human sentences we want ("ready to
  merge but comments to address" = `Approved` + 💬) with no explosion.
- **Fold PR sub-status into `SliceState`** (e.g. `Wip(PrStatus)`). Rejected: it
  would ripple through every `match slice.state` and the `SliceState::BOARD`
  column mapping for no gain. Keeping it additive leaves the state machine and
  its precedence (`Blocked > Wip > Ready`) intact.
- **A dedicated "redundant PRs" state** when a Slice has 2+ open PRs. Rejected as
  YAGNI: two styled PR badges (with their differing statuses) already are the
  "close one" signal; the lower status shows which to close.

## Consequences

- PR sub-status is a **pure derivation over current GitHub facts**, consistent
  with `SliceState`. `Ready to merge` therefore has no representation to keep in
  sync — it is recomputed at render time.
- The classification GraphQL query grows per PR (`isDraft`, `reviewDecision`,
  `mergeable`, `statusCheckRollup`, `reviewThreads`). Still one query per page,
  well within rate limits.
- These fields join the board's change-detection (`BoardSnapshot::same_facts_as`),
  so an approval landing, CI turning green, or a conflict appearing **repaints
  the card live** — the intended behaviour, at the cost of more frequent,
  meaningful repaints (CI is the most-changing signal). Excluding PR sub-status
  from change-detection was rejected: it would show stale review state and defeat
  the feature.
- Only true conflicts (`mergeable = CONFLICTING`) and settled CI failures
  (`FAILURE | ERROR`) are decorated; merely being behind the base branch
  (`BEHIND`, auto-updatable) and pending checks are deliberately not flagged, to
  avoid transient noise.
