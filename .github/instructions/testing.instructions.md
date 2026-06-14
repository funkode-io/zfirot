---
description: Test seams and what makes a good test in Zfirot
applyTo: "crates/**/*.rs"
---

# Testing

- Primary seam: the `GitHubPort` trait. Test use-cases against a **fake**
  `GitHubPort` (and `SecureStorePort`) returning canned data — deterministic,
  offline, no live GitHub or keyring.
- Pure `SliceState` derivation: table-driven unit tests covering precedence, Done
  hiding, two-tier classification, and `## Parent` / `## Blocked by` prose
  fallback parsing.
- Reusable components: verify with fake props and asserted callbacks; do not test
  Dioxus rendering internals.
- Test external behaviour, not implementation details. The live GraphQL client
  and keyring adapter are integration/manual, not in the unit suite.
