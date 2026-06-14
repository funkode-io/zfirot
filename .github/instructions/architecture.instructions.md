---
description: Clean-architecture layering and binding domain decisions for Zfirot
applyTo: "crates/**/*.rs"
---

# Architecture

Clean architecture in a Cargo workspace, one crate per layer. Dependency rule
points inward: `presentation` → `application` → `domain`; `infrastructure` adapts
`application` and is never imported by `domain` or `application`.

- `domain` — read models (`Prd`, `Slice`), locally-owned aggregates as serde
  state + commands, and the pure `SliceState` derivation. No dependencies on
  other layers. Pure and serde-serialisable.
- `application` — use-cases, authorization, and the port traits (`GitHubPort`,
  `SecureStorePort`). Depends on `domain` and the port traits, not on concrete
  infrastructure.
- `infrastructure` — GitHub GraphQL client, OS secure store (keyring), and local
  persistence. Implements the port traits (dependency inversion).
- `presentation` — Dioxus. Submodules: `api` (the seam calling `application`),
  `components` and `pages` (generic, reusable), and a nested `desktop/` for
  desktop-specific components/pages and the binary entry. Web/mobile presentations
  may be added later beside `desktop/`.

## Binding rules

- `Prd` and `Slice` are **read models** projected from GitHub each poll — GitHub
  is the system of record. They are NOT aggregates. (ADR 0002.)
- The only locally-owned state is the tracked/recent projects, the last-opened
  project, settings, and the credential reference. Model it as serde state +
  commands; the replay framework is deliberately deferred. (ADR 0002.)
- `SliceState` (Ready / WIP / Blocked / Done) is a **pure function** over current
  GitHub data, never an event. Precedence: Blocked > WIP > Ready; Done (closed)
  is hidden. Ready = blockers closed AND no open linked PR AND no assignee.
  WIP = open linked PR (closing reference). Blocked = >= 1 open blocked-by.
- Read parent (sub-issue) and blocked-by (dependency) relationships from GitHub's
  native links when present, and fall back to parsing the `## Parent` /
  `## Blocked by` issue-body sections otherwise.
- Reusable `components` are **callback-only**: they never call APIs, they take
  callbacks as props, so they can be previewed and tested without GitHub.
- No backend in v1. Read the GitHub GraphQL API directly. (ADR 0001.)
