# AGENTS.md

Zfirot is a desktop dashboard (Dioxus + Rust) that visualises agent-driven coding
work tracked on GitHub: which PRDs are open, and which Slices are Ready, WIP, or
Blocked. v1 is desktop-only and reads directly from GitHub with no backend.

This file is a concise overview and a router. Read it first, then
[CONTEXT.md](./CONTEXT.md) (the glossary) and the ADRs in [docs/adr/](./docs/adr/)
before starting an issue. Use the project's vocabulary from CONTEXT.md in all
code, commits, and issue updates.

## How to use these docs

Detailed, topic-scoped guidelines live in `.github/instructions/`. Copilot
auto-injects each one when the file you are editing matches its `applyTo`
pattern; other agents should open the relevant file directly. **Load a guideline
only when it is relevant to your current task.**

| Task / file type | Guideline |
|---|---|
| Any Rust file (`crates/**/*.rs`) | [architecture](.github/instructions/architecture.instructions.md), [error-handling](.github/instructions/error-handling.instructions.md), [testing](.github/instructions/testing.instructions.md) |
| Infrastructure / application (`crates/{infrastructure,application}/**`) | [data-auth-classification](.github/instructions/data-auth-classification.instructions.md) |
| Presentation (`crates/presentation/**`) | [ui](.github/instructions/ui.instructions.md) |
| Branching / committing / PRs | [git-workflow](.github/instructions/git-workflow.instructions.md) — load manually |

## At a glance

- **Architecture:** clean architecture in a Cargo workspace, one crate per layer
  (`domain`, `application`, `infrastructure`, `presentation`); dependencies point
  inward. `Prd` and `Slice` are read models projected from GitHub (not
  aggregates). `SliceState` is a pure function: Blocked > WIP > Ready, Done
  hidden. No backend in v1. See the architecture guideline + ADR 0001 / ADR 0002.
- **Data & auth:** GitHub GraphQL only; fine-grained PAT in the OS secure store.
  Two-tier issue classification with a confirm-and-label action. See the
  data-auth-classification guideline.
- **UI:** Dioxus + daisyUI + Iconify; home (recent projects) -> Kanban board;
  callback-only reusable components. See the ui guideline.
- **Testing:** primary seam is the `GitHubPort` trait with fakes; pure
  `SliceState` derivation is table-tested. See the testing guideline.
- **Errors:** a single `AppError` categorised by what the caller can do, with
  structured chaining. See the error-handling guideline.

## Out of scope for v1

Backend; web/mobile presentations; launching docker containers that run agents on
Ready Slices; chat integrations (Teams, Telegram); rewriting the upstream
`to-prd` / `to-issues` skills.

## Skills (evaluation phase)

The upstream `to-prd` / `to-issues` skills are intentionally left unchanged during
a company evaluation, so they emit only the `ready-for-agent` label and prose
`## Parent` / `## Blocked by` sections — no `prd`/`slice` labels and no native
sub-issue/dependency links. The app therefore reads native links when present and
parses the prose otherwise, and offers a confirm-and-add-label action as the
manual tagging mechanism. Migrating the skills to emit labels and native links is
a deferred follow-up driven by the ADRs.
