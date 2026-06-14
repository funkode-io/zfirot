---
description: GitHub GraphQL data source, PAT auth, freshness, and issue classification
applyTo: "crates/{infrastructure,application}/**/*.rs"
---

# Data, Auth & Classification

## Data & auth

- Data source: GitHub **GraphQL** only. One query per project returns its issues
  with parent, dependencies, assignees, labels, and linked-PR state. Mutations:
  assign self (`addAssigneesToAssignable`) and add a classifying label.
- Auth: a user-supplied fine-grained **Personal Access Token** stored in the OS
  secure store. Scopes: Issues read/write, Pull requests read, Contents read.
- Freshness: manual Refresh + a configurable background poll (default ~60s) +
  a "last updated" timestamp.

## Classification (two-tier)

- Tier 1 (confident, automatic): `prd` label -> PRD; native PRD parent or
  `slice`/`ready-for-agent` label -> Slice.
- Tier 2 (heuristic, suggested): unlabeled issues scored by the planning-skill
  template headings (Problem Statement + User Stories -> PRD; What to build +
  Acceptance criteria / Blocked by / Parent -> Slice), surfaced with a
  "looks like a PRD/Slice — confirm?" badge and a confirm-and-add-label action.
- Tier 3: no match -> Unclassified, shown inline in "other open issues".
