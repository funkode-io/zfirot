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
- **Do not commit the compiled stylesheet.**
  `crates/presentation/assets/tailwind.css` is generated from Tailwind + daisyUI
  at build time — `dx serve`/`dx bundle` auto-run the Tailwind watcher (Dioxus
  0.7), and `make css` regenerates it for plain `cargo run`. It was deliberately
  untracked in #85 and is gitignored, so agents should **never** `git add -f` it
  and issues/PRs must not carry a "rebuild and commit `tailwind.css`" step. Just
  write the daisyUI/Tailwind utility classes in the `.rs` components; the CSS is
  produced by the toolchain, not by hand.
- Reusable `components` are **callback-only**: no API calls; take callbacks as
  props so they can be previewed and tested without GitHub. Desktop-specific
  components/pages live under `presentation/desktop/`.

## Signals across `.await` (borrow-safety)

**Never hold a Dioxus signal/resource read or write guard across an `.await`.**
A guard from `.read()`, `.write()`, `.peek()`, or `&*signal` keeps the signal's
internal `RefCell` borrowed; if the code awaits while it is held, a concurrent
re-resolve of the same signal (another effect, a background loop, or a `reload`
bump) re-borrows it and the app **panics with `AlreadyBorrowed`** — a real race
that crashed the board when Refresh was clicked quickly (#116 / #118).

The trap is easy to miss because an `if let` / `match` **scrutinee temporary
lives for the whole block**:

```rust
// WRONG — the peek() guard is held across the await below
if let Some(View::Board { repo, .. }) = &*view.peek() {
    let repo = repo.clone();
    do_network(&repo).await; // panics if `view` re-resolves meanwhile
}

// RIGHT — extract owned data in a tight scope, drop the guard, then await
let repo = match &*view.peek() {
    Some(View::Board { repo, .. }) => Some(repo.clone()),
    _ => None,
};
if let Some(repo) = repo {
    do_network(&repo).await;
}
```

Rules:

- Clone/copy the owned values you need out of a signal, in a `let`/`match` scope
  that ends **before** any `.await`. Never `.await` inside an `if let`/`match`
  whose scrutinee borrows a signal.
- Set in-flight/debounce guards (e.g. `board_refreshing`) **synchronously
  before** `spawn`, not inside the spawned task — otherwise fast repeat events
  slip through the check-then-set gap.
- This is enforced: `clippy.toml` lists the guard types in
  `await-holding-invalid-types`, so `clippy -D warnings` (CI) fails the build if
  a guard is held across an await. Do not silence it — restructure the code.
