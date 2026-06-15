---
description: Dioxus presentation conventions, navigation, and styling
applyTo: "crates/presentation/**/*.rs"
---

# UI

- Home screen lists recently-active projects (by last push, top 5–10) with a
  "Show more" and an empty state. Selecting a project opens its Kanban board. The
  board is organised as one **swimlane per PRD** (a horizontal lane whose header
  links to the PRD issue), with the Ready | WIP | Blocked state columns *inside*
  each lane; Slices with no PRD fall into a trailing "No PRD" lane (the
  "other open issues" bucket). Swimlane-per-PRD was chosen over grouping within
  each column so each product area reads as a self-contained board. The
  last-opened project is persisted on-device and reopened on launch.
- Styling: daisyUI (Tailwind). Icons: Iconify (Tailwind integration), using the
  **Lucide** set via the `@iconify/tailwind4` plugin — reference icons as utility
  classes, e.g. `icon-[lucide--layout-dashboard]`.
- Reusable `components` are **callback-only**: no API calls; take callbacks as
  props so they can be previewed and tested without GitHub. Desktop-specific
  components/pages live under `presentation/desktop/`.
