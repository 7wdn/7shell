# Security model

This document states what the code is held to, and how each rule is proven. It is the document
to read before changing anything that touches credentials, host keys, file access or policy.

## Threat model

**What we defend against**

- A hostile or mistaken server: a host key that does not match, an authentication prompt that
  tries to collect more than it should, a server that sends a banner full of escape sequences.
- A hostile *network*: anything that reads or rewrites traffic, and anything that injects a
  challenge into a keyboard-interactive exchange.
- A local attacker with the disk but not the password: the vault file, its backups, and any copy
  of it that ends up somewhere else.
- A compromised interface layer: the renderer is treated as less trustworthy than the core, so a
  malicious script in the webview cannot read secrets or reach the file system by name.
- A credential leaking through a *report*: an error message, a log line, or a bug report that
  happens to contain a password because something formatted it into a message.

**What we do not defend against (and say so)**

- A machine that is already compromised while it is unlocked: anything with the user's
  privileges can read process memory.
- A user who deliberately trusts a key they should not have. The host key prompt is
  informational and can be answered wrongly; we make the wrong answer loud, not impossible.
- Traffic analysis of *how much* is transferred.

## The invariants

Each one is a promise with a test behind it. When a change touches one of these, update the test
in the same commit — a rule whose test no longer exists is a rule that will be broken by accident.

| # | Invariant | Held by | Proven by |
|---|---|---|---|
| 1 | **A secret never leaves the process.** Passwords, passphrases and key material travel inwards only; what travels out is a boolean ("a secret is stored") and a fingerprint. | The wire types in `crates/sevenshell-types`, which have no field for a secret; scrubbing in `crates/sevenshell-error/src/redact.rs` as a second line of defence | `redact.rs` unit tests (private key blocks, `password=`, bearer tokens, secret-looking parameter keys), `envelope.rs` tests, and the DTO field-parity test |
| 2 | **No credential is sent before the host key is approved.** The connection is established in two stages: key exchange, then verification, then authentication. | `sevenshell-ssh` (planned) | A live test asserting the server logs no authentication attempt before approval, using the SSH test lab |
| 3 | **An unknown host key is never accepted automatically, and a changed one is a hard stop.** | `sevenshell-vault` known-host storage plus the SSH connection stage (planned) | Unit tests over the matching rules, plus a lab case that swaps the server's key and requires the connection to fail |
| 4 | **A decryption failure is indistinguishable.** Wrong password, tampered ciphertext, tampered associated data and a corrupt file all produce the same code, the same message and the same work. | `sevenshell-vault` (planned) | A test that swaps in a truncated file, a flipped byte and a wrong password and compares the envelopes |
| 5 | **The vault write is atomic.** Write to a temporary file, flush, rename; an interrupted write leaves the previous file readable. | `sevenshell-vault` (planned) | A test that fails the write halfway and then reopens the original |
| 6 | **The interface never names a local path.** Native dialogs open in Rust; the interface receives an opaque token that dies when the vault locks. | `sevenshell-policy` local grants (planned), and the capability file, which grants the renderer no filesystem or dialog rights | Grant lifetime tests, and `src-tauri/capabilities/default.json` reviewed in the same pull request |
| 7 | **Every privileged or destructive action has a policy decision and an audit event.** | `sevenshell-policy`: the command risk classifier, and the gate in `Core::run_across`. The policy matrix and the audit event are not built. | A policy matrix test: global, group, host and directory layers, most restrictive wins |

**Where each invariant stands.** Ones and twos and threes below are not aspirations; the right-hand
column names something that runs in CI or against the lab.

| # | Status |
|---|---|
| 1 | **Holding.** No wire type has a field for a secret, and the contract test fails if one appears. The scrubbing tests cover private key blocks, credential assignments, bearer tokens, and secret-looking parameter keys. |
| 2 | **Holding.** Opening a session resolves the credential locally, probes the host key with a connection that sends nothing, and only then authenticates. `live_connection.rs` asserts the first attempt ends in a decision, and the SSH lab's server log confirms no authentication attempt. |
| 3 | **Holding.** A key that has changed produces a stop, not a prompt: the outcome carries both fingerprints for a person to compare, and a trusted key that no longer matches ends the attempt. Covered by a live test against a lab server whose key is deliberately wrong. |
| 4 | **Holding for the vault.** A wrong password, a truncated file, a flipped byte in the payload and a mismatched verifier produce byte-identical error envelopes — asserted directly in `sevenshell-vault`'s tests. |
| 5 | **Holding.** Both the vault and the settings file are written through a temporary file, flushed and renamed, and both have a test that a blocked write leaves the previous file readable. |
| 6 | **Holding.** The renderer cannot name a path: choosing a key file opens a native dialog in Rust, and the interface receives a reference, a file name, an algorithm and a fingerprint. The capability file grants no filesystem and no dialog right to the renderer, and a guard test keeps it that way. |
| 7 | **Partly built.** The command risk classifier exists (`sevenshell-policy`) and grades a command line from safe to catastrophic, naming the rules behind the verdict; 73 corpus cases lock its behaviour against drifting looser. The gate is wired where an action is unattended: `Core::run_across` will not run a command whose level needs approval unless the request carries an acknowledgement, and it decides that before it resolves, connects to, or touches a host. What is **not** built: the audit event, and the policy matrix (global, group, host and directory layers). The terminal path is deliberately not gated — a person typing a command is the person deciding — so today the classifier protects batches, not every destructive action. |

A promise with no code is a comment, and comments do not fail CI: an invariant that is not yet
built says so here rather than reading as though it were.

## Rules the code follows

**A vault from an earlier build still opens.** The cryptography never changed — the same KDF
parameters, the same associated data strings, the same nonce layout — so an older file differs only
in where two fields sit. Its salt lived inside the `kdf` record and the algorithm was called
`algorithm`, and the loader accepts both layouts, rewriting the current one on the next save. The
alternative would be to tell someone their readable vault is unreadable, with deleting it as the
only way forward.

**A route is verified link by link.** A jump host is a machine in the middle by design, so it is
verified and authenticated exactly like the destination: its key is compared before anything is
sent through it, and it is opened as a carrier rather than as a terminal, so no shell is left
running on it. A hop that has been rebuilt is a stop, the same as a destination that has been.

**A forward listens only where it was told to.** The listening address is part of the session's
saved configuration and defaults to `127.0.0.1`; nothing in the interface offers a shortcut to
`0.0.0.0`. A forward carries bytes and nothing else: it cannot be used to run a command, and it
stops with the terminal it belongs to.

**Commands nobody typed are a constant, not a string.** The host monitor runs commands on the far
side without anyone typing them, which is the one place in this build where that is true, so it is
held to a narrower rule than the terminal is: there are exactly four of them — the sample, the
process list, the socket list and the look at one process — all constants in
`crates/sevenshell-core/src/monitor.rs`, none containing a redirection that writes or anything that
changes the far side's state, and all riding a connection a person already opened. The pid that
last command takes is a number, formatted as one, so nothing a person types reaches a shell. The monitor cannot connect to anything by itself, because it is
given a terminal's handle and nothing else. Each runs only while something that shows it is on
screen — the panel while it is mounted, a page while it is the tab on screen — and hiding the panel
stops its sampling rather than only its drawing. It is deliberately *not* behind the risk
classifier: that gate exists to catch a command line nobody has read, and these can be read in the
repository. A change to any of those constants is therefore a change to what the product runs on
somebody's server unattended, and belongs in a pull request on its own.

**One read can return a credential, and only when asked.** `/proc/<pid>/environ` holds whatever a
process was started with, which on a server is routinely a token, a key or a database URL. It is read
for one process at a time, when somebody opens that process, and never as part of a sample or a poll:
the poll runs every two seconds forever, and gathering every process's environment on a timer would
be collecting secrets nobody asked to see. What comes back stays in the interface — it is not logged,
it is not in an error detail, and it is dropped when the tab closes.

**A vault nobody can open is not a dead end.** If the password is gone, the interface offers to
rename the file beside itself — `vault.json.old`, then `.2.old`, and so on — and create a new vault
in its place. Nothing is deleted, the way back is a rename, and the interface is told the file's
*name* rather than where it lives. The alternative was a person staring at an unlock screen with no
way forward, which is how someone concludes that a tool has lost their data.

**Two kinds of state, two homes.** Secrets go in the vault and nowhere else. The settings file
holds only what a user would not mind someone reading: theme, language, and later window state.
Keeping that line sharp is what makes it safe to write the settings file with plain JSON and to
read it back without ceremony — and it is why `settings.json` may never grow a field holding a
password, a key, or a path that names something private.

**The one way a secret leaves this process is a file somebody asked for.** Exporting the session
library writes one JSON file through a native save dialog that runs in Rust. The dialog is the only
moment the writing side learns a path, the file is never read back into the interface, and the
interface is told a count and a file name. What that file may contain is a rule rather than a
setting:

- **Credentials travel sealed, or they do not travel.** With a passphrase, the stored passwords and
  key passphrases go inside a payload encrypted under it (Argon2id, then XChaCha20-Poly1305, with the
  derivation cost recorded in the file so a later release can raise it); without one, the file holds
  the sessions and groups and not one credential. There is deliberately no third form: no argument to
  `sessions_export`, and no shape the interface can send, produces a file with a password in the
  clear. A file like that cannot be told apart from one that is safe to mail, by the only person who
  could still stop it.
- **A sealed file says nothing about what is inside it.** No session count, no group name, no user
  name — a payload and the parameters needed to open it. The associated data binds it to this format,
  so a ciphertext lifted out of a vault file does not open here, and this does not open as a document.
- **A passphrase that worked is remembered for the session, in memory alone.** Exporting twice in one
  sitting means typing it once, so the length rule is not quietly argued down by having to retype it.
  What comes back is only ever the passphrase of a write that actually happened — a file dialog
  somebody closed is not one — and it is held in a module variable: not the store, not
  `localStorage` or `sessionStorage`, and never anywhere near the vault or the settings file. The
  import question is never prefilled, in either direction.
- **The export with no passphrase says what it is missing, before it is written.** The dialog's second
  answer is confirmed once more with the consequence spelled out: the logins stay on this machine and
  will have to be entered again on the other one.
- **Reading is more forgiving than writing.** An export written before that rule existed — a plain
  file with the passwords in it — still imports, secrets and all. It is the only copy of a library
  somebody may no longer be able to reach, and refusing it would punish them for having used the
  feature as it was.
- Importing is the same door in reverse and is narrower. A chosen file's ids are replaced with fresh
  ones where they collide, references only ever point at things inside the same file, and nothing
  already in the vault is modified or removed — a file that came from somewhere else can add to the
  library but never rearrange it. A file that is sealed stops the import on a question rather than
  failing it: where that file is waits in the shell between the two calls, so the interface is still
  never handed a path, and a wrong passphrase is said in the field it was typed in.

The bundle is built in `sevenshell-vault`, which already holds the secrets, rather than in the host
that moves the bytes; what travels up is a summary. The values are wiped from memory when the bundle
is dropped, and the type never implements `Debug`.

**The same rule holds for what comes next.** A local backup and a cloud sync are both "a copy of the
library leaving the machine", so they are held to the rule above: sessions and groups always,
credentials only inside a payload sealed under a passphrase. `ExportProtection` is the shape to reuse
— a `bool` would make "write the passwords into a file anyone can read" a thing a caller could ask
for by getting one argument wrong.

**Secrets are scrubbed, not merely avoided.** `crates/sevenshell-error/src/redact.rs` removes
private key blocks, credential assignments and bearer tokens from any string on its way into an
envelope, masks parameter and diagnostic values whose *key* looks like a secret, and truncates
detail at 64 KiB so an error can never become a data dump. The tests include the over-scrubbing
direction on purpose: a masker that mangles ordinary text pushes people to log the raw value
instead, which is worse than the problem it solves.

**Envelopes are built by constructors.** `ShellError` has private fields and named constructors
(`ssh_host_key_mismatch`, `vault_locked`, …). A caller cannot invent a code, and cannot forget to
say whether anything was transmitted — `outcome` is a required part of every constructor that
deals with credentials, and default values are the deliberate, visible kind.

**Errors carry facts, not blame.** `outcome` distinguishes `notSent` from `sent`, because the
first question after a failure is whether the password left the machine. The interface surfaces
it (`ShellError.nothingWasSent`), so the answer does not depend on reading the code.

**Logs are treated as public.** The rule for anything written to a log or an envelope is the
same as for a bug report: assume a stranger will read it.

## Tauri surface

`src-tauri/capabilities/default.json` grants `core:default` and logging, and nothing else. The
renderer has no filesystem and no dialog permission; when file access arrives it will go through
Rust, which hands back a token. The content security policy is set for production and relaxed
only in development, where the dev server and hot reload need it (`csp` and `devCsp` in
`src-tauri/tauri.conf.json`).

## Reporting

See [SECURITY.md](../SECURITY.md).
