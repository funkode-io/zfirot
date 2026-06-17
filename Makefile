.PHONY: dev run build bundle css css-watch fmt lint test check icon hooks

PRESENTATION := crates/presentation

# Compile the Tailwind + daisyUI + Iconify stylesheet into the bundled asset.
css:
	cd $(PRESENTATION) && npm install && npm run build:css

# Recompile the stylesheet on change (run alongside `make dev`).
css-watch:
	cd $(PRESENTATION) && npm run watch:css

# Rasterise the ZF monogram (assets/logo.svg) into the bundled window/app icon
# (assets/icon.png). Uses macOS QuickLook + sips, so it needs no extra tooling.
# QuickLook flattens the SVG onto white, so round-icon-corners.py then restores
# the transparent corners macOS expects. Rerun after editing logo.svg.
icon:
	cd $(PRESENTATION)/assets && \
	qlmanage -t -s 512 -o . logo.svg >/dev/null && \
	sips -s format png logo.svg.png --out icon.png >/dev/null && \
	rm -f logo.svg.png && \
	python3 round-icon-corners.py icon.png

# Start the desktop app in dev mode (hot-reload) via the Dioxus CLI.
# Compiles the stylesheet first so styling is up to date. Loads .env (if present)
# so ZFIROT_GITHUB_TOKEN reaches the dev-only env secure store, avoiding repeated
# OS keychain prompts across rebuilds.
dev: css
	set -a; [ -f .env ] && . ./.env; set +a; dx serve --package zfirot --platform desktop

# Run the app once without the Dioxus CLI.
run: css
	set -a; [ -f .env ] && . ./.env; set +a; cargo run --package zfirot

# Build the whole workspace.
build:
	cargo build

# Build a standalone, optimised macOS app you can run without the toolchain.
# Compiles the stylesheet first, then produces a .app (and .dmg) under
# target/dx/zfirot/bundle/macos/macos/. Open the .app or drag it to /Applications.
bundle: css
	dx bundle --release --package zfirot --platform desktop
	@echo "App bundled under target/dx/zfirot/bundle/macos/macos/ — open Zfirot.app or drag it to /Applications."

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

# Install the version-controlled git hooks (sets core.hooksPath to .githooks).
# Run once after cloning; the pre-push hook then runs `make check` before every
# push. Bypass in an emergency with `git push --no-verify`.
hooks:
	git config core.hooksPath .githooks
	@echo "Git hooks installed: pre-push runs 'make check'."
