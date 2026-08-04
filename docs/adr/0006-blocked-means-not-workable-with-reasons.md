---
status: accepted
---

# Blocked means "not workable", carrying a ranked reason

`Blocked` meant exactly "has at least one open *blocked by* dependency", so the
only Slices the board could show as unpickable were the ones GitHub happened to
link. A Slice that is deferred, awaiting triage, or waiting on something that is
not an issue at all was therefore either invisible or — worse — shown as
**Ready**, offering an Agent work that cannot be done. We widen **Blocked** to
mean **not workable for any reason**, and give every Blocked Slice a **Blocked
reason** drawn from five signals: `Dependency` (an open blocked-by issue),
`External blocker` (a prose `## Blocked by` entry naming no issue), `Needs
triage`, `Deferred`, and `Labelled blocked` (the last three from labels). The
reasons are **ranked by how much they say about what must change** — `Dependency`
> `External blocker` > `Needs triage` > `Deferred` > `Labelled blocked` — and
that single rank drives both the chip on the card and the order within the
Blocked column, so a card's badge and its position can never disagree. The board
keeps four states and three columns; a Blocked card may now legitimately show
**zero blocker badges**.

## Considered options

- **A separate `Parked` / `Icebox` state**, rendered in the Blocked column but
  counted apart. Rejected: the board answers exactly one question — "what can I
  pick up?" — and every non-Ready, non-WIP card answers it "not this one". A
  second state re-splits what the reader does not need split, and doubles the
  `match slice.state` surface for a distinction a chip already carries.
- **A fourth column.** Rejected for the same reason, at higher cost: it spends
  horizontal space in every lane on cards that are, by definition, not what the
  user came to look at.
- **A second count in the summary strip** ("Blocked 9 · 4 undecided").
  Rejected: it reintroduces the two-state reading we merged away. Ordering the
  column by closeness-to-workable conveys the same thing positionally — the top
  of the column frees up soonest, the bottom needs a human decision.
- **Ignoring the `blocked` label.** Seriously considered, because the label is
  *sticky*: someone applies it, the real blocker lands, nobody removes it, and a
  genuinely Ready Slice hides forever — and a stale hide is the expensive
  failure for a tool whose job is finding grabbable work. Accepted anyway, but
  ranked **lowest** and given its own chip, so a stale label reads as "Blocked
  because someone said so" and is diagnosable on the board rather than invisible.
- **Showing every applicable reason as a chip.** Rejected as noise: the
  motivating issue carries three reasons at once, and the two lower ones add
  nothing the top one has not said. They live in the card's tooltip.

## Consequences

- Only `Dependency` is **self-updating** — it disappears by itself when the
  blocking issue closes. The other four are human-authored and go stale silently.
  This asymmetry is now load-bearing: it is why the reasons are ranked as they
  are, and why an open PR (live evidence that work is happening) outranks a
  sticky reason.
- A Blocked card must render with no blocker badges. The card's dependency-badge
  row can no longer assume `state == Blocked` implies a non-empty `blockers` list.
- `External blocker` is only detectable because the planning skills emit a prose
  `## Blocked by` section. It is the one signal that needs **no label discipline
  whatsoever**, so a repo that labels nothing still gets its unworkable Slices
  off the Ready column. That makes prose parsing a permanent part of the read
  model, not a migration-era fallback.
- Three more repo-vocabulary labels (`deferred`, `needs-triage`, `blocked`) are
  hardcoded alongside `prd`/`spec`/`slice`/`ready-for-agent`. Consistent with
  existing practice, and one more bet on a shared label vocabulary.
- The precedence exception for **Stacked PRs** — where an open PR outranks even a
  `Dependency` reason, because the PR is deliberately branched on its blocker's
  PR — is recorded in the glossary but **deliberately not built yet**. Until it
  is, precedence stays flat (`Blocked > Wip > Ready`), which means a correctly
  stacked PR still reads as Blocked with an out-of-order warning. That is a known
  false alarm, accepted to keep `SliceState` derivable from a Slice's own fields
  rather than from board-wide branch topology.
