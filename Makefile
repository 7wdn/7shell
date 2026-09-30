# Thin wrappers over the pnpm scripts, for people who reach for `make` first.
# The scripts in package.json are the authority; if the two ever disagree, the scripts win.

.DEFAULT_GOAL := help
.PHONY: help install dev dev-web build icons check check-guards test test-rust lint types fmt fmt-check rust-test rust-lint clean

help: ## Show the available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

install: ## Install the interface dependencies
	pnpm install

dev: ## Run the desktop application
	pnpm dev:tauri

dev-web: ## Run the interface alone in a browser (no backend)
	pnpm dev

build: ## Build the interface bundle
	pnpm build

icons: ## Redraw and regenerate the application icons
	pnpm icons

check: ## Run every gate: guards, interface and Rust
	pnpm check

check-guards: ## Run the descriptor, translation, version and origin checks
	node scripts/sync-descriptors.mjs --check && node scripts/check-i18n.mjs && node scripts/reconcile-versions.mjs --check && node scripts/check-provenance.mjs && node --test "scripts/tests/*.test.mjs"

test: ## Run the interface tests
	pnpm test

test-rust: ## Run the Rust tests
	cargo test --workspace

lint: ## Lint the interface
	pnpm lint

types: ## Type-check the interface
	pnpm typecheck

fmt: ## Format the interface sources and scripts
	pnpm fmt

fmt-check: ## Check formatting without writing
	pnpm fmt:check

rust-lint: ## Lint the Rust workspace (warnings are errors)
	cargo clippy --workspace --all-targets -- -D warnings

clean: ## Remove build output
	rm -rf apps/desktop/dist target
