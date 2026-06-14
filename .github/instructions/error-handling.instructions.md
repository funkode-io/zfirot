---
description: AppError constructors, chaining, and structured context
applyTo: "crates/**/*.rs"
---

# Error Handling

Adapted from the team's `error-handling.instructions.md`. v1 has no backend, so
the "client boundary" here is the presentation/UI layer rather than server
functions.

- Define domain errors as a single `AppError` type in `domain`. Categorise errors
  by **what the caller can do**, not by where they came from.
- Map infrastructure errors (GraphQL client, keyring) to `AppError` at the
  boundary via `From` / `.into()`, rather than leaking concrete error types
  upward.
- Error **messages must be client-safe** — they may be shown in the UI. Do not
  embed raw GitHub responses, tokens, or internal details in messages. Log full
  structured context with `tracing`.
- `AppError::internal` must be masked: its `Display` returns a generic
  "Internal error" so implementation details never reach the UI.

## Constructors (pick by what the caller can do)

| Constructor | When |
|---|---|
| `invalid_input(msg)` | Bad data from the caller (validation, missing fields) |
| `not_found(msg)` | Requested entity does not exist |
| `conflict(msg)` | State precondition violated (already assigned, already labelled) |
| `business_rule_violation(msg)` | Business rule violated, not a simple conflict |
| `internal(msg)` | Unexpected infra/IO failure the caller cannot fix |
| `unauthorized(msg)` | Missing/invalid token (PAT absent or rejected) |
| `forbidden(msg)` | Authenticated but lacks permission (token scope too narrow) |
| `unavailable(msg)` | Downstream (GitHub) temporarily unavailable |
| `rate_limited(msg)` | GitHub rate limit exceeded |

## Chaining

Attach structured context instead of formatting values into the message:

- `.with_operation("Module::function" | "CommandVariant")` — name the failing
  operation, especially in low-level/infra code.
- `.with_context("key", value)` — attach every significant input (e.g.
  `"repo"`, `"issue_number"`, `"pr_number"`). snake_case keys; do not repeat the
  message; do not embed values in the message string.
- `.with_source(e)` — attach the underlying error to preserve the chain; never
  `format!` the cause into the message.

Conventional order: constructor → `.with_operation` → `.with_context` →
`.with_source`.

```rust
// WRONG — swallows the chain and bakes inputs into the message
AppError::internal(format!("Failed to load board for {}: {}", repo, e))

// CORRECT — stable message, queryable fields, intact chain
AppError::internal("Failed to load board")
    .with_operation("LoadBoard::run")
    .with_context("repo", repo)
    .with_source(e)
```

## Display vs Debug

- **`Display` (`{}`, `.to_string()`)** — human-readable message only; this is what
  the UI shows. `internal` errors always display generically.
- **`Debug` (`{:?}`)** — full tree (message, operation, context fields, source
  chain) for local diagnosis.
- In `tracing` macros use Debug for errors: `error = ?e` (not `%e`). Non-error
  display values (repo, ids, urls) may use `%`:
  `tracing::warn!(repo = %repo, error = ?e, "poll failed")`.
