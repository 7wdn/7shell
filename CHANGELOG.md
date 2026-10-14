# Changelog

All notable changes to 7Shell are documented in this file. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-10-14

The terminal gets the two things a highlight was always for — taking it, and using it — and three
corners that had one action each have the rest: the vault gate opens onto what past libraries exist,
the session list opens a file manager of its own, and the monitor is a button rather than a trip
through the settings.

### Added

- **A selection in the terminal has actions of its own.** A highlight raises three buttons under the
  point it stops at — copy, hand the text to the shell, find — and the right-click menu carries
  **Paste selection** beside Paste: what is selected reaches the shell the way a paste does, without
  the clipboard and without a copy in between, and the highlight is dropped once it has been handed
  over. The magnifier opens the find field over the selection rather than empty, and that work is
  also where the bar's match count and match marks started working at all: the search addon only
  reports results when it was given colours to draw them with, and xterm keeps that drawing behind
  its proposed-API switch. The buttons hang from the end of the last line the highlight covers, and
  a selection the person scrolls out of the drawn rows takes them away rather than leaving them
  pointing at nothing.
- **The vault gate has ways out, and past libraries come back.** The archived vaults beside the live
  one — `vault.json.old` and its numbered siblings — are listed by name and never by path, tried with
  their own password, made the live vault again, or read into the vault that is open; a switch is two
  renames, so the library in use is archived rather than replaced, and nothing about an archive
  leaves this process. The foot of the gate carries two quiet ways out, with the archive-and-reset
  question asked once in a dialog that says what survives. A refusal under a password field goes when
  the value it was about is edited, the eye on the field stays put through an error, and the title
  bar belongs to the window while the vault is shut — the mark, the version, the three window
  buttons, and nothing drawn from a locked store.
- **The session list opens a file manager of its own.** Its menu offers two things to open whatever
  the workspace setting says a session looks like: a terminal in the shape the setting gives it, and
  the file manager in a tab of its own — a place to browse from rather than the shell's companion.
  Opening a terminal for a session that is already open asks about a second tab, and the answer can
  be remembered and taken back on the Workspace page; clicking a session's row is the other thing and
  never asks. Tabs about one session are numbered so they can be told apart (`lab`, `lab (1)`), per
  session and per kind. Two bugs went with it, both about a browser a tab did not open: a terminal's
  panel no longer adopts another tab's browser and closes it, and the file manager does not stand in
  for a panel or the other way round.
- **The monitor is one click from anywhere.** It was reachable from its settings switch and from its
  own close button — which is only there while the panel is — and it samples only while it is shown,
  so putting it away is also how the sampling stops. The button sits right of Batch execution and
  marks its state the way the transfers button does.

### Changed

- **The workspace overview reads from the top.** The page the setting draws starts at its top now
  rather than centring; Common operations stands above Quick start with its four buttons side by
  side where the width is there and two by two where it is not. The tip explaining where the library
  is, the history card, and the version stamp at the foot are gone — what was sent lives in the
  command library, and the title bar already says which build this is.
- **The library's new-session button stands on its own.** The `+` was one control in a row of five
  view toggles, which is where nobody finds the panel's whole reason; it is a labelled button on the
  left now, carrying the words the home page gives the same action, and Import / Export is an icon in
  the row it left. The default sidebar width grew with the type so the English label is not
  truncated, and the header row is drawn at the size of the rows it heads.
- **The early-days disclaimers are gone.** The READMEs, `SECURITY.md`, the guide and the home view no
  longer carry a block saying the application is early, unaudited and unreleased. What stays is what
  is fact rather than posture: the installer note about code signing and notarization, the route for
  reporting a vulnerability, and the pointers to the security model.

### Fixed

- **The webview's own keys are off, and the browser keys with them.** WebView2 handled a handful of
  the browser's keys before the interface saw them: Ctrl+F opened its find bar over the settings page
  with the caret in its own box, and beside the terminal's find widget, which appeared at the same
  moment; F5 and Ctrl+R reloaded the page and took the open tabs and their terminals with them;
  Ctrl+P, the zoom keys and F12 came along. The switch is the platform's own and leaves the keys that
  edit text alone, and the terminal's find key was never the browser's — xterm claims it first, so
  the widget opens exactly as before. Windows only, because Windows is the only one of the three
  webviews that answers these keys for itself.
- **The platform's own autofill is off, and the field history with it.** The webview kept a
  suggestion history for ordinary text fields and offered it back under every user, host and port a
  session form has — values from outside the session, the person's own among them. The history lives
  in the webview's own data folder; the window now turns it off at the source, which is also where
  the platform's documentation points when it warns that a per-field `autocomplete="off"` is not
  always honored.
- **A double-click on a control takes no text with it.** While a menu is open its scroll lock has the
  whole application out of hit testing, so the second press of such a gesture lands on the document,
  and the platform's word selection then takes the first selectable text anywhere in the window — a
  selection planted over the library search's placeholder, which Copy handed out as if it had been
  typed. A selection that begins on the document is refused at the moment it starts, which is before
  anything is painted; a word inside a real field still selects, and the dialog's own lock upgraded
  with the dropdown library.
- **A password field shows one eye.** WebView2 draws a reveal button into every password field of its
  own accord, so the fields that offer an eye themselves — the gate, the archived-vaults dialog, the
  sync secrets — had two controls at two weights over one masked value. The built-in one is hidden,
  the same way the search field's clear button is.
- **A question is chrome, and a field inside one stays text.** A confirmation dialog could be dragged
  across and what it said came away as a selection; questions refuse selection like the rest of the
  chrome now, and a field inside one — the export passphrase — stays text, which is said once in the
  stylesheet rather than at each field.
- **The macOS Intel disk image builds.** 0.1.2's Intel leg built the application and then died in
  `bundle_dmg.sh` — mounting a volume and asking Finder to lay the window out, neither of which a
  runner does reliably. The bundling is tried three times now, detaching the volume a failed attempt
  left mounted before each retry, and nothing is masked: a real error fails the same way three times.

## [0.1.2] - 2026-10-13

The update channel, end to end: what is published is now something the window can show, download and
install — and the three fixes the first installed builds turned up while that was being built.

### Added

- **The window can be kept on top.** A pin between the release entry and the transfers button holds the
  window above the others for as long as it is on. The state is the window's own: the toggle asks the
  window and then draws what the window says, so a window that refuses is drawn unpinned rather than
  lit over a promise it did not keep — and nothing about it is written down, because a window that came
  back pinned after a restart is one somebody has to unpin to see anything behind it. The release entry
  also moved to the left of that pin: what is published, whether this window stays on top, and then the
  buttons that open pages.
- **The update centre: what is published, and the four things to do about it.** Opening the release
  entry in the title bar (or the update block on the About page) opens a window that asks the channel
  again as it opens, says what is published over what is running, and draws the release notes. From
  there a version can be **downloaded in the background** — the work continues with the window closed,
  and the entry in the title bar carries the percentage — and installed later with **restart and
  update**, which is the moment the window is actually replaced. **Ignore this version** stops a
  release being mentioned anywhere until it is taken back on the About page, and **open the download
  page** hands the release's own page to the system's browser.
- **A release and an installation are two steps now, in the shell as well as on screen.** The archive
  is downloaded and verified when somebody asks for it, and waits there — keyed by the version it was
  signed for, so a later check cannot have an older archive installed under a newer name. A call that
  installs without a download still downloads first, so both paths work.

### Changed

- **The update check waits for the window.** It used to run while the interface was still starting;
  now it runs a few seconds after the window is on screen, and asks again — after a minute, five and
  fifteen — when the channel could not be read, because "the network is not up yet" is not an answer
  and a failed check should not look like "you are up to date". The retries belong to the check, so
  the page about the build keeps its own button for asking by hand.
- **The page about the build describes; the update centre acts.** Its install button opened a download
  of tens of megabytes with no way to say "not now" or "not this version"; it now leads to the window
  where those are the choices.

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
