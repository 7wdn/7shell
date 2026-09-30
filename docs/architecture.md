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

**Current state:** the crates that exist are `sevenshell-error`, `sevenshell-types`,
`sevenshell-platform` (where data lives, including the settings file), `sevenshell-vault` (the
encrypted store), `sevenshell-ssh` (host key verification, authentication, a terminal channel),
`sevenshell-session` (the registry of open terminals), `sevenshell-core` (the order of operations)
and `sevenshell-desktop` (the window and the command surface). `sevenshell-policy` and the host
crates for a command line and an agent interface arrive with the features that need them, so that
nothing exists without a test.

## Opening a session

The order is the security property, so it is worth reading once:

```
session_open(id)
   │
   ├─ 1. read the session and its credential from the vault     ← local only; a session with no
   │                                                              stored secret fails here
   ├─ 2. probe the host key, on a connection that sends nothing  ← no username, no credential
   │
   ├─ 3. compare with what is trusted for that host and port
   │       unknown  → answer `hostKeyUnknown` and stop
   │       changed  → answer `hostKeyMismatch` and stop           ← a stop, not a warning
   │
   └─ 4. open the real connection, which verifies the key again  ← a key that changes between
          inside itself, then authenticates                        the probe and here is refused
```

Step 4 re-verifying is not redundancy: the probe is a separate connection, and the only key worth
trusting is the one the authenticating connection actually received.

Trusting a key that step 3 reported is a separate call (`known_host_trust`), after which the
interface asks to open again. Nothing stays half-open while a person thinks about a fingerprint.

## Terminal events

The core hands bytes to the shell, which turns them into two window events. The payloads are the
shell's own contract, and the interface's mirror of them lives in `types/session.ts`:

| Event | Payload |
|---|---|
| `terminal:data` | `{ handle, base64 }` — bytes, not text: a UTF-8 character that arrives in two chunks must survive |
| `terminal:closed` | `{ handle, reason? }` — the server's own reason, sanitized and capped |

One subscription serves every pane: `lib/terminal/bus.ts` receives both events for the whole
application and fans them out by handle. A terminal that ends is kept in the interface with its
scrollback and a reconnect action, rather than disappearing.

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

## The shell

The window is undecorated on Windows and Linux (`set_decorations(false)` from
`src-tauri/src/window_chrome.rs`), so the interface draws its own title bar, its own three window
buttons and its own drag region. macOS keeps its native chrome in the overlay style, because the
traffic lights and their behaviour are the platform's to own; the title bar insets its content
instead of imitating them.

Dragging and double-click-to-maximize come from Tauri's built-in `data-tauri-drag-region`
handling rather than from a listener of ours: the attribute is on the bar and the empty space in
it, and interactive elements are skipped by the built-in handler, so a button in the drag region
is still a button. Adding a double-click handler would toggle twice and look like nothing
happened.

| Piece | Where |
|---|---|
| Window chrome, per-platform decision | `src-tauri/src/window_chrome.rs` |
| Window controls, the composable they use | `apps/desktop/src/components/shell/`, `src/composables/useWindowControls.ts` |
| The window permissions those need, and nothing else | `src-tauri/capabilities/default.json` |

## Where data lives

Two kinds of state, two homes, decided once:

- **Non-secret preferences** (theme mode, language, and later window state) live in
  `settings.json` in the platform configuration directory, owned by `sevenshell-platform`. It is
  versioned, written atomically, refuses a file from a newer version with `STORE-SCHEMA-NEWER`,
  and is treated as readable by anyone who can read the home directory.
- **Secrets** will live in the vault, which arrives with the SSH layer. Nothing secret may ever be
  written to `settings.json`.

Theme is a three-state choice — `system`, `dark`, `light` — with `system` as the default and a
real mode rather than a placeholder: the interface follows the operating system while it is open
and keeps following it as it changes. A language is only recorded once someone chooses one, so a
fresh install follows the system language instead of freezing it.

## Doctests on Windows

Doctests are an illustration in this workspace rather than a compiled check, and the reason is
worth knowing before adding one:

`tauri-build`, which the desktop crate has to run, writes a stub `msvcrt.lib` into its build output
directory, puts that directory on the link search path, and passes `/NODEFAULTLIB:msvcrt.lib` for
its own binary. Rustdoc merges the workspace's link search paths into the doctest binary, so a
doctest ends up resolving `msvcrt` to the 82-byte stub and fails with `LNK2001: unresolved external
symbol __CxxFrameHandler3` — a link error that says nothing about the code being tested.

So: examples in documentation are marked ` ```text ` on purpose, and the same behaviour is covered
by an ordinary test beside the code. If a compiled example is genuinely needed, it belongs in
`tests/`, where it links normally.

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
