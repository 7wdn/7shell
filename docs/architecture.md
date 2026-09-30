# Architecture

The project ships one binary to three kinds of user: someone at a desktop, someone in a
terminal, and an agent acting on someone's behalf. Everything below follows from that.

## Layers and the direction of dependency

```
        src-tauri            sevenshell-cli      sevenshell-mcp      sevenshell-web
       (desktop host)         (planned)           (planned)           (planned)
              \                    |                   |                    /
                               sevenshell-api          (planned: the operation surface)
                                     |
                              sevenshell-core          the only orchestration layer
        ┌──────────────┬──────────────────┼───────────────────┬──────────────────┐
  sevenshell-       sevenshell-      sevenshell-        sevenshell-        sevenshell-
   session            sftp             vault              policy            profiles
        │                                                  │
  sevenshell-ssh ──────────────┬──────────────────┬─────────┘
        │               sevenshell-types    sevenshell-error   sevenshell-platform
        └───────────────────────┴──────────────────┘
```

Rules, enforced by `scripts/tests/architecture.test.mjs`:

1. Dependencies point downwards only. A lower crate never depends on a higher one, and nothing
   depends on a host.
2. **The core must not know that Tauri exists.** Hosts depend on the core; the reverse would make
   the command line and the agent interface impossible without a rewrite.
3. `sevenshell-error` and `sevenshell-types` are leaves, depending on little more than `serde`.
4. Each concern has exactly one home. A new crate is a decision, not a default: rank it in the
   guard test and say why in the pull request.

**Current state:** `sevenshell-error`, `sevenshell-types` and `sevenshell-desktop` exist. The
other crates arrive with the features that need them, so that nothing exists without a test.

## The four patterns worth knowing

### 1. One error contract, everywhere

Every failure is a `sevenshell_error::ShellError`: a stable `code`, a `message_key` plus
parameters to localize it, an `origin` (subsystem, adapter, peer), an `outcome` that answers
*"was anything sensitive already sent?"*, a scrubbed `detail` and named diagnostics. Fields are
private and filled only by named constructors, so a code and its message key cannot drift apart;
the catalog is one macro invocation, so a new code cannot be added without its key.

The interface mirrors it in `apps/desktop/src/lib/backend/errors.ts`, and
`packages/app-tests/contracts.test.ts` fails when the two sides disagree — in both directions.
`crates/sevenshell-error/tests/wire_format.rs` pins the JSON shape.

### 2. Descriptors are the single source of truth

`descriptors/**/*.yaml` declares protocols, authentication methods and the capability
vocabulary. A generator produces `descriptors/descriptors.manifest.json` and
`apps/desktop/src/types/generated/protocols.ts`; Rust reads the manifest at compile time and
generates enums and capability tables from it.

Capabilities are consumed in three places: the interface decides which panes exist, the policy
layer decides which actions may be attempted, and the agent interface will decide which tools to
offer. Adding a protocol is a YAML file, not a change to every `match`. Details in
[descriptors.md](descriptors.md).

### 3. One facade in front of the backend

Everything in the interface imports `src/lib/backend/api.ts`. Behind it, `tauri.ts` holds the
typed wrappers and `api.ts` decides which transport answers, answers "no backend" with a real
envelope rather than an undefined-function error, and normalizes whatever is rejected with. When
a second transport arrives it is selected inside that module and nothing else changes;
`scripts/tests/frontendBoundary.test.mjs` keeps it that way.

Interface folders: `components/<domain>/`, `composables/`, `stores/`, `lib/<domain>/`,
`i18n/locales/<language>/<domain>.json`, `types/` (with `types/generated/` written by the
generator and excluded from formatting).

### 4. Settings and data are versioned, and the interface never names a path

Persisted documents carry a `schemaVersion` and a migration chain; data written by a newer
version is refused with `STORE-SCHEMA-NEWER` rather than guessed at. The interface receives
tokens for files the user picked through a native dialog, never paths — which is also why the
Tauri capability file grants no filesystem or dialog permission to the renderer.

## Startup

A command that needs a service that is not ready yet answers `APP-NOT-READY`. The readiness gate
lands together with the first service that needs it (the vault and its migration chain), so that
the mechanism is never dead code: there is nothing to gate yet.

## Build and check

`pnpm check` runs every gate in order of speed, and CI runs the same steps. The interesting ones:

| Command | What it protects |
|---|---|
| `pnpm check:descriptors` | The generated Rust tables and TypeScript types match the YAML |
| `pnpm check:i18n` | Every key used by descriptors, Rust or the interface is translated in every locale |
| `pnpm check:provenance` | No trace of the material this project was designed from (local rules file; skipped where absent) |
| `pnpm verify:versions` | `version.json` is the only version number anyone edits |
| `pnpm test:guards` | Dependency direction and the frontend boundary |
| `cargo clippy -- -D warnings` | Warnings are errors |
