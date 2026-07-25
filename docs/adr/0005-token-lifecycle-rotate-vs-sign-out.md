# Token lifecycle: Rotate vs Sign out, and the Invalid/Under-scoped split

The app distinguishes two credential-failure meanings instead of one. An
**Invalid token** (Unauthorized / 401) is cleared and routed to the paste-token
screen. An **Under-scoped token** (Forbidden / 403) is **kept** and offered for
**Rotate** (an atomic swap: the old token stays valid until a new one is
validated and saved), because it is still usable elsewhere and the driving
scenario is a *growing* set of required grants — a new feature needs a new
permission, so the existing token is under-scoped, not bad.

**Rotate** (UI: "Change token") preserves all local state, since it is the same
Viewer with a new credential. **Sign out** wipes account-scoped state (token,
tracked repos, last-opened project, cached recent projects, cached boards) but
keeps UI preferences (theme, view mode), since those are not tied to the Viewer.
Sign out is guarded by a confirm dialog; Rotate needs none because it cannot lose
a working token.

These credential-failure meanings are first-class across **every** GitHub
operation (board load, project list, `assign_self`, `add_label`, and the viewer
fetch), surfaced as a route at load time and an actionable banner mid-feature —
both leading to Rotate for the Under-scoped case, carrying the specific missing
permission so the required-permissions list can highlight the exact grant to add.

## Considered options

- **Single "clear on any auth failure" (the prior behaviour)** — rejected: it
  evicts a *valid* token, and the board with it, the moment one feature hits a
  missing scope, stranding the user with no path to re-grant.
- **Two-meaning split (chosen)** — costs an Unauthorized/Forbidden distinction at
  every call site and a distinct Change-token view carrying a Cancel path, but
  matches the real scenarios: a dead credential vs. a live but under-scoped one.
