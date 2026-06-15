---
description: Dioxus presentation conventions, navigation, and styling
applyTo: "crates/presentation/**/*.rs"
---

# UI

- Home screen lists recently-active projects (by last push, top 5–10) with a
  "Show more" and an empty state. Selecting a project opens its Kanban board
  (columns Ready | WIP | Blocked, cards tagged by PRD, plus an
  "other open issues" bucket). The last-opened project is persisted on-device and
  reopened on launch.
- The board is grouped into **Lanes**, one per PRD: each lane is a horizontal
  swimlane with a header linking to its PRD Issue, wrapping that PRD's
  Ready | WIP | Blocked columns. Slices with no parent PRD collect in a trailing
  "No PRD" lane. The grouping is a pure domain function
  (`domain::group_into_lanes`); the per-card "PRD: …" tag is dropped because the
  lane header now carries the PRD. Lanes follow first-seen PRD order; Done Slices
  and empty lanes are omitted.
- Styling: daisyUI (Tailwind). Icons: Iconify (Tailwind integration), using the
  **Lucide** set via the `@iconify/tailwind4` plugin — reference icons as utility
  classes, e.g. `icon-[lucide--layout-dashboard]`.
- Reusable `components` are **callback-only**: no API calls; take callbacks as
  props so they can be previewed and tested without GitHub. Desktop-specific
  components/pages live under `presentation/desktop/`.
