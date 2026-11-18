# Changelog

All notable changes to 7Shell are documented in this file. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-11-18

The virtual-machine release. VMware Workstation machines on this computer are part of the library
now: imported with their UUID as identity, shown with their real power state, and driven from the
same workspace and the same agent tools as everything else. The session list also learned to
interleave groups and sessions, and a directory with thousands of entries can be narrowed by name.

### Added

#### Virtual machines

- Scan the Workstation inventory and import machines as sessions, into a chosen group or a new one.
  A machine keeps its UUID when its `.vmx` is renamed or moved.
- Running, suspended and stopped are drawn apart, so a suspended machine resumes instead of
  booting. The workspace starts, resumes, stops and suspends; the power tool adds reset, pause and
  unpause. A lock left by a machine that is not running can be taken over after confirmation.
- SSH, or RDP on Windows, opens from the machine entry. The guest account and password are the
  session's own vault slot.
- Snapshots can be listed, taken, gone back to and deleted; one can be cloned, linked or full, into
  a folder chosen through a host token. `.vmtx` templates are created and deployed the same way.
- Through VMware Tools: list and check paths, copy files both ways, run programs and scripts,
  create, rename and remove, list processes, read the environment, kill a process, and install or
  upgrade Tools.
- Configuration covers CPU, memory and cores, disks to create or extend, network and serial
  adapters, SATA and NVMe controllers, shared folders, VProbes (enable, load, reset), configuration
  entries, device connection and runtime variables.
- On the host: virtual networks and port forwards, console screenshots, key sequences that need no
  guest account, and the Workstation REST service for machine reads, MAC-to-IP maps and network
  creation.
- Host locations are opaque tokens whose paths stay in the vault; imported machines are reached by
  session id. The vault schema is now 13.

#### Agent tools

- 17 built-in VMware tools cover inventory, status and power, snapshots and clones, guest reads and
  writes, console keys, configuration, host networking, templates and images, screenshots and the
  REST reads. Destructive calls stop for the same approval and are written to the same audit log as
  every other tool.

#### Files

- The open directory can be filtered by name: `Ctrl+F` or the search button turns the address bar
  into a field, matching files and folders without case; Enter hands the keyboard to the results,
  Escape puts the whole directory back, and walking into another directory drops the filter.

#### Sessions

- Groups and sessions now share one order. A dragged row lands beside the row it was released on, a
  group can sit between two sessions, pinned groups keep their band, and existing vaults are
  numbered the way they were already drawn.

### Fixed

- Workstation 26H1 installs under 64-bit `Program Files`; that folder is asked before the legacy
  one, a registry entry only counts when the binary is really there, and a missing module command
  names `VMCLI_PATH`.
- Guest operations read the password from the session's vault slot rather than the global one, which
  made an unlocked vault answer `VAULT-LOCKED`.
- Snapshot revert and delete resolve the uid the query prints (`-d` for children), a clone creates
  its destination folder, and reset is hard so it works before Tools answers.
- Console capture uses `vmcli MKS`, which works before a guest has Tools or an account.
- The Workstation REST service no longer returns `.vmx` paths in its answers.

### Notes

- The vault is at schema 13. A 0.2.0 build refuses a vault written by 0.3.0; downgrading means
  restoring a backup.
- The installers are not code-signed and the macOS build is not notarized. Windows SmartScreen and
  macOS Gatekeeper will warn; check the SHA-256 in the release notes.
- The browser edition is attached to the same release for amd64 and arm64 as `*-web.tar.gz`, built
  against the glibc 2.28 baseline. It is not part of the desktop installers.
- Signed update archives and `latest.json` are attached when the signing key is configured. The
  update channel changes only when the release is promoted.

## [0.2.0] - 2026-11-17

The browser-host release. The interface now runs in a normal browser from a loopback host, so
machines without the newer WebKit — older or locked-down Linux, including UOS and Kylin — can use
the application without a desktop shell. Terminals, files and tab handling were reworked around
how people actually use them: one shared command history, tabs that survive a dropped connection,
and a file panel whose header is a single compact strip.

### Added

#### Browser host

- A loopback-only host (`sevenshell-web`) that serves the built interface and answers the same
  command names over HTTP and a WebSocket. One transport layer, `ipc.ts`, selects the backend, so
  the interface is the same code in both hosts.
- The vault, sessions and groups, known hosts, keys, SSH terminals with host-key decisions, SFTP
  browsing and text editing, transfers, settings and application facts all work in the browser.
- The host monitor, batch execution, the command library, container and Kubernetes workspaces,
  sync, key management, SFTP upload and download, drag-and-drop upload, and container and pod file
  browsers are part of the browser surface as well.
- The assistant and external MCP servers are available in the browser: the same tool registry,
  approval prompts and audit log as the desktop, an MCP listener and stdio bridge, OAuth sign-in
  for external servers with token refresh and reconnect, resources, prompts and resource
  templates, per-session agent policy, bounded context, and the same run limits (turns, one tool
  per turn, result truncation).
- Terminal, assistant and MCP events are delivered in the desktop's envelope, so every listener
  behaves identically in both hosts.
- Downloads started in the browser appear in the transfer panel with progress, and Stop cancels
  them. ZMODEM sends and receives through the browser's own file doors.

#### Files

- The address bar is one row: breadcrumbs navigate, and the empty part of the bar or `Ctrl+L`
  turns it into a field for typing a path. Enter or Go travels; Escape or leaving the field returns
  to the breadcrumbs without navigating.
- The panel header is a compact icon strip: refresh the directory, go up, new file, new folder,
  download (only once something is selected), upload, and upload from the clipboard.
- Directory bookmarks: named remote directories kept in the settings file and reachable in one
  click from the path bar's menu.
- The panel follows a `cd` typed at the keyboard, accepted from completion, sent from the command
  bar, or sent from the command library.
- A file browser opens a connection of its own when the terminal's connection has no room for
  another channel, instead of failing when the server reaches its per-connection limit.
- Opening a binary or oversized file as text reports the editor's answer rather than a failed
  transfer.

#### Terminals and tabs

- The tab menu offers duplicating a tab, reconnecting it, reconnecting every closed terminal,
  disconnecting while keeping the tab, closing it, closing the others, closing left, closing right
  and closing all.
- Reconnecting revives the tab in place: the same id, position and title, the same scrollback, and
  the new session writes below what the old one left on screen.
- Command history is shared by every terminal window. Lines run at a prompt are recorded — except
  at password, one-time-code and confirmation prompts — deduplicated, persisted, and offered back
  from the command bar. The history list closes when a click lands anywhere else.
- The host monitor mounts only where a sample can ride an SSH connection, and comes back by itself
  when a monitorable session is on screen again. Switching it off still means switched off.
- An SSH channel failure names the channel rather than a session, which is what made a full
  connection read as an unopenable terminal.

#### Containers and clusters

- A container page keeps only the namespace picker a runtime supports, and a cluster page keeps
  its endpoint and connection state. The session's name and kind live in the tab, not in a second
  heading inside the page.

### Fixed

- Browser-host events reached listeners without the desktop envelope, which left terminals blank
  and assistant deltas undefined.
- An external MCP server reconnects after its OAuth token is refreshed.
- A disconnected terminal tab keeps its last screen and its state; it no longer shows the
  open-failure card underneath, and reconnecting no longer replaces the tab with a new one.

### Notes

- The installers are still not code-signed and the macOS build is not notarized; Windows
  SmartScreen and macOS Gatekeeper will warn. Check the SHA-256 in the release notes.
- The browser host is a separate binary (`sevenshell-web`) and is not part of the desktop
  installers; it is built and packaged on its own. The Windows, macOS and Linux desktop
  installers are unchanged in shape.
- Signed update archives and `latest.json` are attached when the release signing key is
  configured. This is the first release that can seed the update channel once it is promoted.

## [0.1.0] - 2026-11-13

The first public release: a desktop SSH and operations client for Windows, macOS and Linux.
Sessions, terminals, files, containers, clusters and host readings share one application, and
everything the application knows stays on this machine.

### Added

#### Sessions and terminals

- Saved sessions in an encrypted vault, with groups, search, import/export and one passphrase.
  Argon2id derives the key and XChaCha20-Poly1305 encrypts the contents; the passphrase is never
  stored.
- SSH with a password, private key, keyboard-interactive answers or the system key agent. Jump
  hosts, local port forwarding (`-L`) and a SOCKS5 proxy (`-D`) are part of the saved session.
- Telnet, raw TCP, serial, local shells (PowerShell, Command Prompt, Git Bash and the Unix login
  shell) and WSL on Windows.
- Tabs, search, links, keep-alive, reconnect, asciicast v2 recording, 190 colour schemes,
  typography, cursor settings and keyword highlighting seeded with 22 rules.
- Host keys are verified before authentication. An unknown key is a decision; a changed key stops
  the connection.

#### Files

- A two-pane SFTP workbench: create, rename, delete, edit text in place, upload and download.
- A transfer queue with pause, resume and retry from where a transfer stopped. Transfers outlive
  the tab that started them, and a directory can travel as one archive over SSH.
- WebDAV and FTP/FTPS folders in the same workbench, SCP fallback when a server has no SFTP
  subsystem, and ZMODEM inside the terminal with the system file dialogs.

#### Operations

- A host monitor for load, CPU, memory, swap, disks, processes, network and sockets, sampled over
  the connection that is already open. Nothing is installed on the server.
- Batch execution: one command on several hosts, a result per host and a risk verdict before it
  runs.
- A command library with `#1`–`#5` slots, history and completion.
- Sync sessions and commands to a WebDAV server, or to a snippet on GitHub, Gitee or GitLab.
- RDP on Windows through the system Remote Desktop client.

#### Containers and clusters

- Docker, Podman, nerdctl and WSLC: containers, images, logs, stats, an exec terminal, files and
  image-archive import, on this machine or through an SSH session.
- Kubernetes: kubeconfig contexts, resource lists and watches, pod logs, exec, pod files, metrics
  and YAML editing, directly or through an SSH session.

#### Assistant and MCP

- An in-app assistant for Anthropic, OpenAI-compatible endpoints and local models.
- An MCP server for external clients. The assistant and MCP clients use the same tool registry,
  risk rules, approval prompts and audit log; credentials stay in the vault.

#### Packages

- Windows x64 and ARM64: MSI, NSIS, portable ZIP and a WebView2-offline NSIS installer.
- macOS Apple silicon and Intel: DMG, plus an update archive when the release signing key is
  configured.
- Linux x64 and ARM64: AppImage, deb and rpm.
- `latest.json` covers all six platform keys when the release signing key is configured.

### Notes

- The installers are not code-signed and the macOS build is not notarized, so Windows SmartScreen
  and macOS Gatekeeper will warn. Check the SHA-256 in the release notes before running one.
- A portable Windows build keeps its vault, settings and MCP bridge in a `data` folder beside the
  executable.
