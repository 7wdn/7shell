# Changelog

All notable changes to 7Shell are documented in this file. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
