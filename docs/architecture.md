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
   session            sftp             vault              policy            sync
        │                                                 │                   │
  sevenshell-ssh ──────────────┬──────────────────┬───────┴─────────┬─────────┘
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
encrypted store), `sevenshell-policy` (what may be run, and how dangerous it is), `sevenshell-ssh`
(host key verification, authentication, a terminal channel), `sevenshell-session` (the registry of
open terminals), `sevenshell-sftp` (the far side of the file workbench), `sevenshell-sync` (a copy
of the library that travels: what a snapshot holds, and where it goes) and `sevenshell-core` (the
order of operations), with `sevenshell-desktop` as the window and the command surface. The host
crates for a command line and an agent interface arrive with the features that need them, so that
nothing exists without a test.

`sevenshell-sync` sits at rank 3, beside `sevenshell-ssh`, because it is the same kind of thing: a
crate that deals with something outside this machine and takes what it carries from the vault. It is
below the core so the core can drive a backup the way it drives a connection, and it knows nothing
about a window — which is what will let a command line run `sshell backup` one day without a second
implementation of the file format.

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

## Terminal colours

The terminal does not take its colours from the interface theme. It wears one of two palettes —
`.terminal-dark` or `.terminal-light` — and which one is a setting: **follow the interface** (the
default), always dark, or always light.

Both halves of that are deliberate:

- **Always dark is a real choice.** A dark terminal in a light interface is what most people who
  use light interfaces actually want, so the two decisions are kept apart instead of tied together.
- **A palette is a set, not a background.** Every ANSI colour differs between the two: bright
  yellow on white is unreadable, and so is dark blue on black. `lib/terminal/palette.spec.ts` fails
  when either palette stops defining one of the twenty colours the terminal reads, because xterm
  silently falls back to its own default and the result looks like a rendering bug rather than a
  missing token.

The colours themselves stay in the stylesheet like every other colour in the interface. The pane
carries its palette class, reads the computed values from itself rather than from the document, and
hands each one to the browser first — xterm does not parse `oklch()`.

## Routes and forwards

A jump host and a port forward are the same primitive twice: a channel opened on an established
session behaves like a socket, so a second SSH connection can run over it and a listener on this
machine can feed bytes into it.

```
session with jump = [A, B]
   │
   ├─ connect to A directly, verify A's key, authenticate as A
   ├─ open a direct-tcpip channel on A to B, run the SSH handshake to B over it,
   │  verify B's key, authenticate as B
   ├─ probe the target through B (sending nothing), compare with what is trusted
   └─ open the real connection to the target over B, which verifies the target's key again
```

Three things about that are deliberate:

**Every hop is verified, not just the last one.** A route is only as trustworthy as its weakest
link, and a hop that is not checked is a machine that can read everything passing through it.

**A hop is opened as a carrier, not as a terminal.** Asking a jump host for a shell would leave a
process running on it that nobody asked for and nobody would think to look for. `Purpose::Carrier`
is what keeps a jump host's logs clean.

**One decision at a time.** A hop whose key has never been seen ends the attempt with a decision
naming *that host*, so the same prompt covers a route of any length: trust it, ask again, and the
next unknown host asks in its turn. Nothing stays half-open while somebody reads a fingerprint.

Forwards belong to a session and come up with its terminal. A forward that cannot listen — a port
already taken, most often — does not fail the terminal: the reason travels back in the open outcome
and is shown next to the forwards that did come up. Each forward counts the connections it carried
and the ones it could not, because "the tunnel is up but nothing works" is exactly the state that
needs explaining, and a library has no business writing to somebody's log.

## Remote files

A file browser rides on a connection of its own — or on a terminal's, when one is already open for
that session, because one connection carries both and a second authentication is a second thing for
the server to log for no reason. Its traffic goes through a channel opened as the `sftp` subsystem
on a session that was opened as a *carrier*: a jump host or a destination that nobody logs into does
not get a shell for the privilege of listing a directory.

Two rules the interface depends on:

- **Local paths are chosen in Rust.** A download opens a save dialog and an upload opens a file
  picker, both in the shell, and the interface receives the id of the transfer that does the work.
  It never names a path (invariant 6).
- **A transfer outlives the command that started it.** The record keeps its progress, its state and
  its task; the interface polls the list while something is running, which costs a read of a few
  counters and saves a second event channel whose only job would be to move a number.

**A transfer never writes the name it was asked for until it has the whole thing.** Both directions
write beside their target — `notes.txt.7shell-part` — and rename it into place once every byte has
arrived, so a half-written file never wears the name somebody checks, on either end. The same fact
seen the other way round is what makes a retry worth pressing: the part file's length *is* how much
arrived, so the next attempt continues from there rather than sending a gigabyte again.

Two rules keep a resume from joining two versions of one file. A transfer's **first** attempt clears
a part file it did not make, because a part file from an earlier run of the application cannot be
checked against what it came from; only a retry within the same transfer continues. And a retry
continues only while the source is unchanged — same size, same modification time — which the core
watches because it is the layer that knows what the source was. Cancelling removes the part file
(somebody said stop); failing keeps it, because the next thing they will try is the retry button.
A failure nobody retries leaves one file, clearly named, that is theirs to delete.

Three things can be done to a transfer — hold a running one, let a paused one go, try a stopped one
again — and the row has one button that says which of the three it is.

### Two panes

The browser shows a directory tree beside the listing, and the tree **follows** the listing rather
than driving it: wherever the right side navigates, `ancestorsOf` opens exactly the branches that
make that place visible. That is what keeps the two panes from disagreeing about where you are, and
it is why the tree has no opinion of its own about the current directory.

Two consequences worth knowing:

- **Branches are read lazily and cached.** Collapsing and reopening a branch costs nothing.
- **A change anywhere drops the whole cache.** A folder created in one place changes what a folder
  elsewhere holds, so the revision counter invalidates every cached branch and the open ones re-read
  — one round trip each, which is the price of never showing a stale tree.

The layout is a preference (`tree` or `list`), because a narrow window wants the listing alone.

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

## The host monitor

The panel to the right of the workspace is a live reading of whichever connection is on screen, and
four decisions shape it:

1. **A sample rides a connection that is already open.** `monitor_sample` takes a *terminal's*
   handle, not a session id, and `Core::sample_host` runs one command on the connection the
   registry already holds. A session with no terminal open has nothing to sample, and the panel
   says so rather than opening a connection of its own — a monitor that authenticated every couple
   of seconds would be a worse neighbour than one that shows nothing.
2. **One command per sample.** `SNAPSHOT_COMMAND` in `crates/sevenshell-core/src/monitor.rs` is a
   fixed, read-only `/bin/sh` script that reads `/proc`, `uname`, `/etc/os-release`, `ps` and
   `df`, with each group introduced by a `@@name` marker line. A round trip costs more than the
   readings do.
3. **Counters travel; rates do not.** `HostMetrics` carries cumulative values — CPU ticks, bytes
   through an interface — and the difference between two samples is computed in the interface
   (`lib/monitor/rates.ts`). A single counter cannot be a rate, and "the average since boot"
   presented as the current load is the one answer that is always wrong and always plausible.
4. **A section the host did not answer is `null`, never zero.** `/proc` is Linux; another system
   answers some of these questions and not others. The panel draws a dash where a number would be,
   which is visibly different from a measurement.

**The panel is the glance; the pages are the read.** Each card opens a tab about the same host —
system, processes, network — and a page is a *view*, not a connection: it holds a session, resolves
a terminal for it at read time, and is drawn as "not connected" when the session has none. That is
what lets a page be left open across a reconnect instead of pointing at a dead handle.

Two of the pages ask questions of their own, on their own slower clocks, and only while they are
the tab on screen: the process page reads the table with command lines, processor time and program
paths (`Core::list_processes`), and the network page reads the host's sockets (`Core::list_sockets` —
listening and connected together, since that is what the host's `ss` or `netstat` answers and the
page shows them as two views of one list). The process page also reads one process in full when
somebody opens it — command line, directory, environment — which is the only read in this build that
can return a credential, and the reason it happens on a click rather than on a clock.

The process table's CPU column is the interval between two reads, taken from the kernel's own
counters, because `ps`'s own column is an average over each process's whole life: a service that has
been up for a week reads the same number every time it is asked, and a page that refreshes and never
changes is a page nobody believes. That is the same rule the host's CPU and the interface rates
follow, and the reason the panel beside it says which reading it is showing.

Everything else on a page comes from the same sample the panel reads, so a number in the panel and
the same number on a page are one reading rather than two that agree.

Sampling runs while something is asking for it, and stops when the last of them has gone: the panel
asks while it is mounted, a page asks while it is the tab on screen, and one poll serves them both.

The commands are the only place in this build where something runs on a remote host without somebody
typing it, so they are held to their own rule — see "Commands nobody typed are a constant, not a
string" in `docs/security-model.md`.

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

### Wire shapes have their own guard

Every type that crosses to the interface is serialized in a test and checked two ways: that it
serializes at all, and that every field name is camelCase at every depth
(`crates/sevenshell-types/tests/wire.rs`). Both have gone wrong:

`#[serde(rename_all = "camelCase")]` on an **enum** renames its *variants*, not the fields inside
them. `OpenOutcome::Opened { session_id, tunnel_failures }` therefore went out as `session_id` and
`tunnel_failures` while every struct around it spoke camelCase. The interface read `undefined`,
iterated nothing, and reported "something went wrong inside the application" — a failure that says
nothing about its cause, which is the worst kind to receive from a test suite that was green.

`rename_all_fields = "camelCase"` is the attribute that fixes it, and the guard is what notices
when somebody forgets it on the next enum with fields in it.

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

The window is also created hidden and revealed only once the interface reports a drawn frame
(`app_ready`, called from the shell component after the theme and the language are settled). An
un-painted webview is an opaque white rectangle — the browser control's own default background,
which no stylesheet can reach, because the stylesheet arrives with the document that has not been
painted — so a window shown at creation spends its first few hundred milliseconds looking like a
broken application. `src-tauri/src/window_chrome.rs` holds a three-second deadline as the way out:
a frontend that never gets that far still ends up as a window somebody can look at and close, and
`tauri.conf.json` gives that path the dark palette's background colour rather than white.

Dragging and double-click-to-maximize come from Tauri's built-in `data-tauri-drag-region`
handling rather than from a listener of ours: the attribute is on the bar and the empty space in
it, and interactive elements are skipped by the built-in handler, so a button in the drag region
is still a button. Adding a double-click handler would toggle twice and look like nothing
happened.

| Piece | Where |
|---|---|
| Window chrome, per-platform decision | `src-tauri/src/window_chrome.rs` |
| The frame the window is revealed on, and the deadline | `apps/desktop/src/App.vue`, `src-tauri/src/window_chrome.rs` |
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
