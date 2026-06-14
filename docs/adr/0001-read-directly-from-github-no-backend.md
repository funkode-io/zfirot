# Read directly from GitHub with no backend (v1)

The first iteration is a desktop dashboard that visualises agent coding work
already tracked on GitHub. We deliberately ship **no backend**: the app reads the
GitHub GraphQL API directly and authenticates with a user-supplied fine-grained
Personal Access Token stored in the OS secure store. We chose this because the
whole point of v1 is to surface what is happening *in GitHub* with the least
moving parts, and a token-in-keychain native desktop app needs no redirect
server or OAuth exchange.

## Consequences

- GitHub is the system of record; `Prd` and `Slice` are read models, not state we
  own. The only locally-owned state is the set of tracked repos, credentials, and
  settings.
- A backend will likely be added later (for chat integrations and the docker
  agent orchestration phase). The architecture keeps GitHub access and state
  derivation in a UI-free layer so a backend can be slotted in without rewriting
  the domain.
- Relationship facts that drive the board — parent (sub-issues) and blocked-by
  (issue dependencies) — have no GitHub timeline history, so they are read as
  current state, never reconstructed as historical events.
- The upstream skills (`to-prd`, `to-issues`) are intentionally left unchanged
  during a company evaluation phase, so they emit neither a `prd`/`slice` label
  nor native sub-issue/dependency links. The app therefore reads native
  relationships when present and falls back to parsing the `## Parent` and
  `## Blocked by` body sections, and offers a confirm-and-add-label action to
  tag issues by hand. Migrating the skills to emit labels and native links is a
  deferred follow-up to be driven by these ADRs.
