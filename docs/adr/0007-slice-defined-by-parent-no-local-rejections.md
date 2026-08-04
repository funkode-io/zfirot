---
status: accepted
---

# A Slice is defined by its Parent, and rejections live on GitHub

Classification promoted an issue to a **Slice** only on tier‑1 evidence: a
`slice` / `ready-for-agent` label, or a native sub-issue link to a PRD. But the
planning skills label only the *workable* Slices, and issues split out of a
ticket after the fact often carry nothing but a prose `## Parent`. Those Slices —
precisely the deferred and undecided ones — never reached the board. We make a
resolvable **Parent** sufficient on its own: an issue whose parent (native, else
the prose `## Parent` section) resolves to a PRD **in the fetched board** is a
Slice, whatever its labels. The parent may be a confident PRD *or* a **Suggested
PRD** (one matching the PRD template headings without the label), which requires
classification to run in **two passes** — classify every issue independently,
then promote children against the resulting PRD set. Suggested PRDs head lanes
like any other, marked unconfirmed, with a positive-only **Confirm** that adds
the `prd` label and a **Veto** that adds `not-a-prd`. The app stores **no local
memory of either decision**.

## Considered options

- **Requiring the parent to be a confident (`prd`-labelled) PRD.** Rejected: the
  planning skills emit only `ready-for-agent`, so in a repo where nobody has
  hand-labelled the PRD, the PRD set is empty and every prose-parented Slice
  stays invisible — the exact bug we set out to fix.
- **Accepting any open issue as a parent.** This is what the existing lane
  resolution already does when it looks a Slice's parent up among all issues, so
  it was tempting for consistency. Rejected: that lookup only ever ran on issues
  *already* confirmed as Slices. Promoting it into the classification rule turns
  a lookup into a policy — a bug's sub-task becomes a Slice on a lane headed by
  the bug, and the board fills with work that is not part of any plan.
- **Treating `deferred` / `needs-triage` as Slice evidence.** Rejected as the
  wrong shape: those labels say something about an issue's *readiness*, nothing
  about whether it is a Slice. A `needs-triage` bug report would be dragged onto
  a lane.
- **A local "dismissed suggestions" store** behind a yes/no confirmation dialog.
  Rejected on two grounds. It breaks the invariant that tracked repos are the
  *only* domain state the app owns locally; and a private negative decision goes
  stale the moment a teammate labels the issue `prd`, leaving local state
  silently fighting GitHub. A `not-a-prd` label is visible to the whole team,
  survives a cache clear, and is reversible by the same mechanism that set it.
- **Leaving opt-out to editing the issue body** (renaming a heading until the
  heuristic stops matching). Rejected as undiscoverable and corrupting: it asks a
  writer to distort a document to control a dashboard, and the obvious attempt —
  demoting `## User Stories` to `### User Stories` — does not even work, because
  the heuristic matches on substrings.

## Consequences

- Classification is no longer a per-issue pure function; the second pass needs
  the board's PRD set. Both passes stay pure and in-memory, so the board-level
  projection remains fully testable offline.
- Promotion now depends on a *heuristic*. If a PRD's body drifts and stops
  matching the heading heuristic, it silently drops out of the PRD set and **all
  of its prose-parented children leave the board together**. Natively linked
  children are unaffected. This coupling is the price of working in unlabelled
  repos, and `Confirm` is the durable escape from it — a labelled PRD never
  depends on its own prose again.
- The board grows to include every prose-parented issue, so a repo with sloppy
  `## Parent` sections gets noisier lanes. The "other open issues" tray shrinks
  to genuine orphans, and the existing confirm-and-label action loses most of its
  original purpose while gaining a new one at the lane header.
- Every user decision the app records is a GitHub label, so two people looking at
  the same repo always see the same board. The app remains, as in ADR 0001, a
  pure reader of GitHub with one small locally-owned exception (tracked repos).
