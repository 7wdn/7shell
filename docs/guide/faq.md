# FAQ

## Where does the application keep its data?

One directory per installation, decided per platform:

| Platform | Directory |
|---|---|
| Windows | `%APPDATA%\com.sevenshell.app` |
| macOS | `~/Library/Application Support/com.sevenshell.app` |
| Linux | `$XDG_CONFIG_HOME/com.sevenshell.app`, or `~/.config/com.sevenshell.app` when that variable is unset |

`SEVENSHELL_CONFIG_DIR` moves the whole directory somewhere else (an empty value is ignored). It has
two real uses: a portable installation that keeps its vault beside the binary, and a test run that
must not touch the real vault, settings or known hosts.

What is inside:

| File | What it holds |
|---|---|
| `vault.json` | Everything secret or private: sessions and groups, stored passwords and key material, the host keys this device trusts, the command library and the suggested words |
| `settings.json` | What is not secret: the theme, language, layouts, fonts, interface scale, panel widths, the shortcuts you have changed, the window's background image reference, and the sync server's address, user name and remote path |
| `downloads.json` | The download folder you chose, and the names used before |
| The background image | Copied into this directory when it is chosen, so moving or deleting the original does not empty the window |
| `recordings/` | One file per recorded session, when session recording is used |

`vault.json` cannot be read without the vault password. There is no recovery: if the password is
gone, the unlock screen can rename the file aside (`vault.json.old`, then `.2.old`) and create a new
vault in its place, and nothing is deleted — but what was in the file stays unreadable. The same
screen lists those renamed files, so one can be tried with a password, switched back to, or read
into the vault that is open; the folder they live in is in the table above.

`settings.json` is plain JSON on purpose: anybody who can read your home directory can read it,
which is exactly why no secret may be written into it. Both files are written atomically — a
temporary file, flushed, then renamed — so an interrupted write leaves the previous file readable.
Both carry a version, and a file written by a *newer* version is refused with `STORE-SCHEMA-NEWER`
rather than guessed at.

## Does the application send anything anywhere?

No account, no telemetry, no analytics. The outbound traffic is:

1. **The connections you ask for** — the hosts you have saved and opened.
2. **A sync server, if you configure one.** Uploads are the file described below, sent to the server
   you named. A timed upload runs only while the window is open, because the timer lives in the
   interface; nothing uploads on a schedule from a process nobody is looking at. A download is never
   automatic.
3. **The update check, the only request the application makes on its own.** It reads the release
   channel on GitHub and sends nothing about you, your machine or what you are working on.
   Settings → About carries the switch ("Check for updates at startup"); the button beside it asks by
   hand whenever you would rather.

## Do my credentials leave the machine?

Only over a connection whose host key you have approved. Opening a session reads the credential out
of the vault locally, probes the host key over a connection that sends nothing — no user name, no
password, no key — compares it with what this device trusts, and authenticates only after that. A
key that does not match stops before anything is sent, and the error envelope says so.

Three further rules hold:

- **No wire type has a field for a secret.** What crosses to the interface is a boolean ("a secret
  is stored") and a fingerprint. The one key-shaped thing that crosses is the public half of a key
  you just made or are exporting, which is not a secret.
- **Logs, errors and events are treated as public.** Private key blocks, credential assignments and
  bearer tokens are scrubbed on the way into an envelope, and parameters whose key looks like a
  secret are masked.
- **The only way a credential leaves as a file is one you asked for.** An export or a backup carries
  the credentials sealed under a passphrase (Argon2id, then XChaCha20-Poly1305) or carries none at
  all — there is deliberately no third form.

## Why has there been no security audit?

Because nobody has performed one: this is an early project and an external audit has not been paid
for or arranged. Nothing in the build claims otherwise — the interface's own notice says it has had
no security review and has never been released, and the [release process](/release-process.md)
requires every release to keep saying so until an audit actually happens.

What does exist is [the security model](/security-model.md), which names each invariant together
with the test that holds it up, and states plainly which parts are not built yet (the audit event
and the policy matrix are the two named there). Reading it is the honest way to judge what is proven.

Suspected vulnerabilities go through the
[security policy](https://github.com/7wdn/7shell/blob/main/SECURITY.md) as a private security
advisory, not as a public issue.

## Does the interface claim more than the code does?

That is the one thing the project makes mechanical rather than promising. Protocols, authentication
methods and capabilities are declared once in `descriptors/**/*.yaml`; a generator produces the Rust
enums and the TypeScript types, and the capability list inside the application is read from those
tables. A capability is added by the change that implements and tests it, so the build cannot offer
a button behind which there is nothing — something the SSH protocol can do but this build cannot yet
sits in the catalog unclaimed.

The practical answer to "can it do X" is therefore the application's own introduction page,
**What this build can and cannot do**. If a release note and that screen ever disagree, the screen
is right and the note is a bug.

## Can I use it with production credentials?

Not yet, and the build says so itself on every empty screen: early development, no security review,
never released. Use it against machines you do not mind losing until a release says otherwise.

## How do I move my sessions to another machine, or keep a copy?

Two ways, both of them yours to start:

- **Import / Export in the session library.** Export writes every session and group to one file.
  With a passphrase, the stored passwords and key passphrases travel sealed inside it; without one,
  the file holds no credential at all. Importing is the same door in reverse — an id that already
  exists here gets a fresh one, references only point at things inside the same file, and nothing
  already in the vault is modified or removed.
- **Sync, in the settings.** A backup holds everything this installation knows — the session
  library, the command library, the words, imported keys, trusted host keys and the settings that
  describe how you work, plus the command history only if you ask for it. It is sealed under a
  passphrase that never reaches the server, so what the server holds is worth nothing without it.

## Where do I report a bug?

Bug reports and contributions: [CONTRIBUTING.md](https://github.com/7wdn/7shell/blob/main/CONTRIBUTING.md).
A report that can be acted on carries the error's **code** (the monospace line under the message),
the platform and version from Settings → About, and what the far side was;
[Troubleshooting](troubleshooting.md) lists what each failure usually means. A suspected
vulnerability is not a public issue: it goes to a private security advisory as described in the
[security policy](https://github.com/7wdn/7shell/blob/main/SECURITY.md).
