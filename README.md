# Zfirot

A desktop app that reads your GitHub Projects directly via the GraphQL API — no
backend — and visualises each board's PRD, lanes, and slices. Built with
[Dioxus](https://dioxuslabs.com/) (desktop).

## Requirements

- **macOS** (Apple Silicon or Intel). Other platforms are untested.
- [Rust](https://rustup.rs/) (stable toolchain).
- The [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/) (`dx`),
  needed for `make dev` and `make bundle`. Dioxus 0.7 auto-runs the Tailwind
  watcher, so no separate Node/Tailwind step is needed for those:

  ```sh
  cargo install dioxus-cli
  ```

- [Node.js](https://nodejs.org/) — only for `make run` (plain `cargo run`),
  which regenerates the Tailwind/daisyUI stylesheet via `make css`.

## Run on macOS

### Option A — standalone app (recommended)

Build an optimised, self-contained `.app` you can run without the toolchain:

```sh
make bundle
```

This produces the bundle under
`target/dx/zfirot/bundle/macos/macos/`:

- `Zfirot.app` — double-click to run, or drag it into `/Applications`.
- `Zfirot_<version>_aarch64.dmg` — a disk image for distribution.

Because the app is not notarised, the first launch is blocked by Gatekeeper.
Open it once via **right-click → Open** (or
`System Settings → Privacy & Security → Open Anyway`); subsequent launches work
normally.

### Option B — run from source

```sh
make run    # regenerate the stylesheet and run once via cargo
# or
make dev    # hot-reloading dev mode via the Dioxus CLI (auto-runs Tailwind)
```

## Authentication

Zfirot needs a **fine-grained GitHub Personal Access Token**. Grant these
repository permissions:

- **Issues**: Read and write
- **Pull requests**: Read-only
- **Contents**: Read-only

Paste the token into the app when prompted. It is stored in the **macOS
Keychain** (never on disk in plain text) and reused on the next launch.

> For local development only, you can set `ZFIROT_GITHUB_TOKEN` in a `.env`
> file. `make dev` / `make run` load it so debug rebuilds skip the Keychain
> prompt. This env shortcut is ignored in release builds.

## Development

```sh
make hooks   # install the pre-push quality gate (run once after cloning)
make check   # fmt --check, clippy -D warnings, and tests (the gate the hook runs)
make css     # regenerate the stylesheet (only needed for `make run`)
make dev     # hot-reloading desktop app (dx auto-runs Tailwind)
```

See [AGENTS.md](AGENTS.md) and [CONTEXT.md](CONTEXT.md) for architecture and
domain language.
