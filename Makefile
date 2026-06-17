.PHONY: dev run build css css-watch fmt lint test check icon hooks

PRESENTATION := crates/presentation

# Compile the Tailwind + daisyUI + Iconify stylesheet into the bundled asset.
css:
	cd $(PRESENTATION) && npm install && npm run build:css

# Recompile the stylesheet on change (run alongside `make dev`).
css-watch:
	cd $(PRESENTATION) && npm run watch:css

# Rasterise the ZF monogram (assets/logo.svg) into the bundled window icon
# (assets/icon.png) used by the desktop window. Uses macOS QuickLook + sips, so
# it needs no extra tooling; rerun after editing logo.svg.
icon:
	cd $(PRESENTATION)/assets && \
	qlmanage -t -s 512 -o . logo.svg >/dev/null && \
	sips -s format png logo.svg.png --out icon.png >/dev/null && \
	rm -f logo.svg.png

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
