# Changelog

All notable changes to 7Shell are documented in this file. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The release channel has a place in the title bar.** An icon that says what the channel said — the
  version itself when a release is published, the state in its tooltip while a check is out or after
  one failed — and pressing it asks, or leads to the page that installs what it found. The question it
  answers is one people ask before there is an answer, so it is there whether or not there is anything
  to say.

### Changed

- **The update check waits for the window.** It used to run while the interface was still starting;
  now it runs a few seconds after the window is on screen, and asks again — after a minute, five and
  fifteen — when the channel could not be read, because "the network is not up yet" is not an answer
  and a failed check should not look like "you are up to date". The retries belong to the check, so
  the page about the build keeps its own button for asking by hand.

### Fixed

- **A second launch started a second application.** Two shells mean two tray icons, two writers to
  one vault file and two sets of live sessions against the same servers. A launch now hands its
  request to the instance already running — which brings its window forward, from the tray if that is
  where it is — and the new process ends without opening anything.
- **A window closed into the tray while the application was still starting came back on its own.**
  The window is created hidden and put on screen when the interface reports its first painted frame,
  or by a three-second deadline when it never does. Both asked whether the window was visible, and a
  window somebody has just closed into the tray is not visible either — so whichever came first undid
  the close, which is why it happened "sometimes", and mostly at startup. The shell records that the
  window was put away on purpose, and only the tray icon (or a second launch) clears that.
- **The terminal ignored its own font settings — in installed builds only.** Everything the terminal
  draws with lives in a stylesheet that xterm writes at runtime: the family and the size of its text,
  the foreground and the sixteen colours, the cursor and the height of a row. A packaged build's
  content security policy blocked it — Tauri adds a nonce to `style-src`, and a nonce turns
  `'unsafe-inline'` off for the whole document — so the stylesheet sat in the page, complete and never
  applied, and the pane drew in the interface's own proportional font at the interface's own size.
  Changing *font size* or *terminal font* changed nothing, and the terminal was monochrome. `tauri dev`
  was never affected, because the page comes from the dev server, which sends no policy at all: every
  test in this repository runs the development application, which is why it shipped this way. Tauri now
  leaves `style-src` alone (`src-tauri/tauri.conf.json`), and `scripts/tests/windowCsp.test.mjs` is
  what keeps it that way.

## [0.1.1] - 2026-10-12

Two things the first build got wrong on Windows, both of them settings nobody had written rather than
code that misbehaved.

### Fixed

- **The installer wore NSIS's own icon.** Tauri's NSIS template takes the installer's and the
  uninstaller's icon from `bundle.windows.nsis.installerIcon`, a setting with no default — so
  an unset one is not the application's icon but the build tool's. Both now point at the project's
  own. The wizard also follows the system's language (English and Simplified Chinese) instead of
  always being English, and shows `7Shell 0.1.1` in its bottom-left corner rather than the copyright
  notice, which is where every other installer puts the product and its version.
- **"Follow the system" did not follow the system.** The interface asked the webview what the system
  was, and WebView2 answers "light" whatever it is — with the system in its dark appearance, the
  answer was light in all three theme modes, including the one where the window itself was dark. The
  shell reads the system's own setting now, and watches it change.
- **The tray icon's right-click menu kept the appearance of the mode before it**, which on Windows is
  read from the system rather than from the application. It follows the application now, at the
  moment a theme is chosen rather than when the settings page is saved.

## [0.1.0] - 2026-10-12

The first build of 7Shell: an SSH client for people who look after a few machines and would rather
the work stayed on their own computer. One window holds a session library, a terminal per
connection, a file workbench beside it and a host monitor on the right. What it knows — sessions,
commands, keys and credentials — is encrypted on this machine behind a passphrase only you hold.
There is no account and no telemetry, and the only request it makes on its own is the update check,
which can be turned off.

**No security review has been done, of this release or of anything before it.** 7Shell handles
credentials and live access to servers, so treat this as something to experiment with and not as
something to put production credentials in yet. `SECURITY.md` says the same, and so does the notice
in the interface footer.

### Added

#### Sessions and connections

- A session library: address, user, authentication, jump hosts and forwards in one saved entry,
  with nested groups, search, a list of what was used recently, and import/export to a file.
- **Host keys are verified before any credential is sent.** The first connection to an unknown host
  probes for its key and stops with the fingerprint for you to accept or reject; a key that has
  changed since it was trusted is a hard stop, not a warning to click past. Trusted keys can be
  reviewed and revoked in the settings.
- Authentication with a password, a private key (encrypted or not), keyboard-interactive answers, or
  a key the SSH agent on this machine already holds — the key material never leaves the agent. The
  Windows agent's named pipe is used when nothing else names one. Pageant is listed as an
  authentication method and marked unsupported: it speaks a protocol of its own.
- Jump hosts, as many per session as needed. Every hop is verified like the destination, and a jump
  host is opened as a carrier rather than a shell, so nothing is left running on it that nobody
  asked for.
- Local port forwarding and dynamic forwarding (a SOCKS5 proxy answered from the far side), coming
  up with the session, counted, and reporting why a port could not be taken instead of failing the
  terminal. Remote forwarding is not built. Agent forwarding has a switch of its own in the session
  editor, but this build does not claim it: what is missing is proof of the far side using it, so it
  is listed among the capabilities that are not claimed yet.
- Terminal resizing with the window, and keep-alive on an idle connection.

#### Terminals

- A tab per session, each with its own font, size and colour scheme. The terminal's palette —
  always dark, always light, or following the interface — is a separate choice from the interface
  theme, because a dark terminal in a light window is a real thing to want.
- Scrollback search, clickable links, and a session that ended kept in place with its scrollback and
  a reconnect action rather than disappearing.
- A completion bar under the terminal: commands, shell built-ins, subcommands, the lines you have
  run before, your saved commands, and paths from the file browser's view of the host.

#### Files

- A file workbench beside the terminal: a directory tree and a listing, six sortable columns,
  multi-row selection, create, rename, delete, and a plain text editor for remote files (no syntax
  highlighting yet).
- Uploads by picking files, by dragging them onto the window, or by pasting them; downloads to a
  folder chosen once so later ones stop asking, or a save dialog every time if nothing was chosen.
- Transfers with a life of their own: hold, continue, retry from where it stopped, stop, remove,
  open the finished file or show it in the folder. A transfer outlives the tab it started in.
- Both directions write beside the target (`notes.txt.7shell-part`) and rename it into place once
  every byte has arrived, so a half-written file never wears the name you are checking. A retry
  resumes only while the source is unchanged, so two versions of one file are never joined.
- A whole folder can travel packed as a single archive in either direction: the far side's own
  `tar` packs a download, and an upload is packed here before it goes.
- A file browser rides on a connection that is already open when there is one, rather than
  authenticating the server a second time.

#### Monitoring and batch work

- A host monitor beside the workspace: load, memory, disk, network interfaces and processes, read
  with fixed, read-only commands over the connection that is already open, so nothing has to be
  installed on the host. Each card opens a page about the same host — system, processes, network —
  and a page is a view, not another connection. Opened process details can include the command's
  environment, which is the one read in this build that can return a credential, so it happens on a
  click and never on a timer.
- Batch execution: one command on many saved sessions at once, with how many run in parallel and how
  long each is given. Every host is connected to before anything runs, so a host needing a decision
  stops the run instead of the command running twice on the others; one host's failure stays that
  host's result.

#### Saved commands and the risk classifier

- A library of saved commands in groups, with placeholders for values, an optional Enter after
  sending, and a trigger word that offers the line while you type. A command goes into a shell that
  is already open — exactly the bytes you would have typed.
- Sending to the terminal in front of you is not gated; sending to several at once is, because
  nobody reads ten shells at the same time.
- A command risk classifier that reads a command before it runs and grades it — safe, caution,
  destructive or catastrophic — with the reasons it reached that verdict shown to you. A command it
  cannot parse is caution, never safe. The batch panel shows the verdict before anything runs, and
  a grade that needs approval waits for it in the core, where a caller without a button meets it
  too.

#### Session recording

- Terminal sessions can be recorded per session, as [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/)
  — the format other players already read. A recording holds what the server printed, not the
  keystrokes, and it is a plain file on disk, which the interface says out loud.

#### The vault and keys

- One encrypted vault, behind one passphrase, holds sessions, groups, saved commands, per-session
  secrets, imported and generated keys, trusted host keys and the credentials for a sync target.
  The key is derived with Argon2id (64 MiB, three passes) with the cost recorded in the file, and
  the whole document is sealed as one XChaCha20-Poly1305 blob, never field by field.
- A wrong passphrase, a truncated file and a flipped byte all produce the same answer, so the file
  cannot be used as an oracle for guessing. A locked vault keeps answering that it is locked.
- Writes are atomic: an interrupted save leaves the previous vault readable.
- Keys can be generated here (Ed25519, ECDSA P-256, RSA 3072) or imported, with a passphrase if they
  have one. The page returns the public line for `authorized_keys` and the fingerprint to compare
  against `ssh-keygen -lf`, and never the private material.
- Sync credentials (a WebDAV password, a gist or snippet token) live in the vault rather than in the
  settings file, and are only written there when you ask for them to be kept.

#### Sync and backup

- A backup of what this installation knows — sessions, groups, saved commands, keys, trusted host
  keys and chosen settings — to a local file, to a WebDAV server (test the address, upload,
  download), or to a private gist or snippet on GitHub, GitLab or Gitee, including a self-hosted
  GitLab address.
- A backup carries credentials sealed or not at all: without a passphrase the secrets never leave
  the machine, and with one they travel inside the sealed payload. The gist and snippet copy is
  always sealed, because "private" there means unlisted.
- An upload can run on its own while the window is open and the vault is unlocked, on an interval
  between one minute and one day. Nothing is uploaded while the vault is locked, and a download is
  never automatic.
- Restoring first shows what it would write, without touching anything. Adding never overwrites: a
  file restored twice makes a second copy unless you turn replacement on.
- An upload refuses to write over a gist or snippet that is not this application's, rather than
  treating a mistyped id as permission.

#### Updates

- An in-app update channel: check by hand or at startup, read which version is published, when it
  was published and what changed, then install and restart on request. Update metadata is verified
  against a signing key embedded in the build, and the check is the only request the application
  makes without being asked.

#### Interface

- Light, dark or follow the system, in English or Simplified Chinese, with the terminal palette kept
  out of that choice. The interface itself comes in a set of colour schemes, light and dark, with a
  custom one whose colours you pick.
- A window the application draws itself: a tab bar that can move edge, wrap or keep its width, and
  group and sort its tabs; panels that float over the workspace or sit in a frame; adjustable
  interface scale, fonts, corner rounding and a background picture; and a tray to come back to when
  the window is put away instead of closed.
- Settings with search, help beside each control, and a reset that says what it will put back.
- A home page that states what this build can and cannot do, read from the same capability tables
  the Rust side compiles against, and a shortcut table listing every key the application reserves.
- Native dialogs stay in the host process and the interface never sees a local path: it receives a
  token for whatever was picked, which is what keeps drag-and-drop uploads and downloads honest.
