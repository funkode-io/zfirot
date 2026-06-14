.PHONY: dev run build css css-watch fmt lint test check

PRESENTATION := crates/presentation

# Compile the Tailwind + daisyUI + Iconify stylesheet into the bundled asset.
css:
	cd $(PRESENTATION) && npm install && npm run build:css

# Recompile the stylesheet on change (run alongside `make dev`).
css-watch:
	cd $(PRESENTATION) && npm run watch:css

# Start the desktop app in dev mode (hot-reload) via the Dioxus CLI.
# Compiles the stylesheet first so styling is up to date.
dev: css
	dx serve --package zfirot --platform desktop

# Run the app once without the Dioxus CLI.
run: css
	cargo run --package zfirot

# Build the whole workspace.
build:
	cargo build

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

# Full quality gate: format check, lints, and tests.
check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test
