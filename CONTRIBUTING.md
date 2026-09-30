# Contributing

Thanks for taking the time. This document covers how to get a change from your machine into
the project without surprises.

## Getting set up

You need Rust (the exact version is pinned in `rust-toolchain.toml`), Node 22 or newer, and
pnpm 11. On Windows, the MSVC toolchain and WebView2.

```bash
pnpm install
pnpm dev:tauri   # the desktop application
pnpm check       # everything the CI runs, minus the platform matrix
```

## What a change must satisfy

1. **`pnpm check` passes.** It runs the descriptor drift check, the translation key check, the
   version check, the origin check, the architecture and boundary guards, formatting, linting,
   type checking, the interface tests and the Rust gates. CI runs the same steps.
2. **Rust tests pass too** — `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
   Warnings are errors here; a new warning is a failed build.
3. **Every user-visible string is a message key with translations in every locale.**
   `pnpm check:i18n` fails when a key is used and not translated, or when the locales disagree
   about which keys exist. Adding English text only is not enough.
4. **The invariants in [docs/security-model.md](docs/security-model.md) still hold.** If a change
   cannot keep one of them, that is a conversation to have *before* the pull request, not a
   footnote in it. Each invariant names the test that proves it, and those tests are expected to
   be updated alongside.

## Layout rules

| Rule | Why |
|---|---|
| A Rust crate may only depend on lower-ranked crates, and only hosts may depend on Tauri. | Keeps the operations callable from a command line or an agent interface later. `scripts/tests/architecture.test.mjs` enforces it; rank a new crate deliberately. |
| The interface imports the backend only through `src/lib/backend/api.ts`. | One place to add a second transport. `scripts/tests/frontendBoundary.test.mjs` enforces it. |
| Anything both languages must agree on goes into `descriptors/`, never into two hand-written lists. | `pnpm check:descriptors` fails when the generated files are stale. |
| Domain code and tests live in domain folders, not at the root of `src/lib/`. | With a few hundred files, "where does this belong" stops being obvious. |
| No file is copied from another project without its licence being compatible and its attribution recorded. | This project is an independent implementation; a copy-paste can silently change that. |

## Commits and branches

Conventional commit subjects, scoped to the area you touched:

```
feat(ssh): verify the host key before authenticating
fix(sftp): keep a partial download when the connection drops
docs(descriptors): explain the planned capability flag
chore(ci): cache the Rust build
```

Scopes in use: `vault`, `ssh`, `sftp`, `terminal`, `sessions`, `policy`, `types`, `error`,
`descriptors`, `ui`, `i18n`, `scripts`, `ci`, `docs`. Branch names: `feat/…`, `fix/…`, `docs/…`.

Explain *why* in the body when the change is not obvious. The commit that says what changed is
useful; the one that says why is the one people read in two years.

## Adding a dependency

New dependencies are held to a high bar, because this project ships a small binary to people's
machines: active maintenance, a compatible licence, and no runtime of its own. Rust dependencies
are checked by `cargo-deny` in CI against an explicit licence allow-list. Prefer the standard
library and the crates already in use.

## Documentation

Proposals and designs for larger changes go in `docs/pips/` (see the template there). Anything
that changes an invariant, a public contract or the layout gets a document before it gets code.
`docs/` describes the project as it is; if you find it describing something that no longer
exists, that is a bug worth fixing.
