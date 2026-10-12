---
layout: home

hero:
  name: 7Shell
  text: An open-source SSH and operations console
  tagline: For Windows, macOS and Linux. A Rust core and a Tauri desktop shell, with the credentials on your machine and no account anywhere.
  actions:
    - theme: brand
      text: Install
      link: /guide/install
    - theme: alt
      text: Getting started
      link: /guide/getting-started

features:
  - title: An encrypted vault
    details: "Passwords, key passphrases, saved sessions and trusted host keys live in one file, opened with a password only you hold. Argon2id derives the key and XChaCha20-Poly1305 encrypts the contents; nothing secret is written to the settings file."
  - title: A host key is checked before anything is sent
    details: "A connection first probes the server's key over a connection that sends nothing, then compares it with what this device trusts. An unknown key is a decision you make. A key that has changed stops the connection."
  - title: Terminals that stay readable
    details: "A terminal per session, each with its own font, colour palette and scrollback. A terminal that ends stays on screen with what it printed and a reconnect action beside it."
  - title: A file workbench, not a file list
    details: "Browse with a directory tree or a plain listing, create, rename, delete and edit text in place, and move files in both directions. The transfer list says which row failed and why, and a stopped transfer is retried from where it stopped."
  - title: Jump hosts and port forwards
    details: "Reach a host through other saved sessions, each hop's key verified and each hop authenticated in turn. Local port forwards come up with their session and count the connections they carried."
  - title: A host monitor without an agent
    details: "Load, memory, network and file systems of the host you are on, sampled over the connection that is already open, with pages for the system, its processes and its sockets. Nothing to install on the server."
---

## Where the project stands

This is an early prerelease. The installers are not code-signed and the macOS build is not
notarized yet, so the operating system will warn about the download — see
[Install](guide/install.md) for what each platform says and what to do about it. No security audit
has happened, and [the security model](security-model.md) marks every invariant with whether a test
proves it today or whether it is not built.

What the build can do is not a claim made on this page: the capability list inside the application
is generated from [descriptors](descriptors.md), and a capability appears there only once the code
behind it exists.

![7Shell with the session library on the left, a terminal open and the file panel beneath it](/screenshots/workspace.png)

*The session library, a terminal and the file panel for the same session.*
