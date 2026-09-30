# 7Shell

**An open-source SSH and operations console for Windows, macOS and Linux.**

*Read this in [Chinese](README.zh-CN.md).*

> **Status: early development.** The engineering foundation, the error contract and the
> descriptor pipeline are in place. Nothing connects to a server yet, no release has been
> made, and the project has not had a security review. **Do not put production credentials
> into it.**

## What it will be

- **Sessions that stay honest about security.** Credentials live in an encrypted,
  password-protected vault. A server's host key is verified *before* anything is sent to it,
  and a key that changed is a hard stop, not a prompt to dismiss.
- **A real terminal.** Multi-tab, resizable, searchable, with a scrollback that survives
  switching tabs, and sessions that stay on screen when they end so you can read what
  happened before reconnecting.
- **A file workbench, not a file list.** Browse, transfer, edit in place, and see why a
  transfer failed instead of watching a progress bar stall.
- **Operations at scale.** Jump hosts, proxies, port forwarding, batch execution against
  several machines, a snippet library, and a command risk classifier that says out loud
  when something is destructive.
- **Small and dependency-free.** A single Rust + Tauri binary. No JRE, no Python, no bundled
  browser runtime.

## What works today

| Area | State |
|---|---|
| Workspace, build, test and check tooling | In place |
| Error contract (stable codes, message keys, secret scrubbing) | In place, with tests |
| Descriptor pipeline (one YAML source → Rust enums + TypeScript types) | In place, drift-checked in CI |
| Encrypted vault (Argon2id + XChaCha20-Poly1305, atomic writes, one unlock failure) | Working, with tests |
| Sessions and groups, saved in the vault, with no secret in any wire type | Working |
| Host key verification: trust a new key, stop on a changed one | Working, tested against a real sshd |
| Terminal: multi-tab, resize, keep-alive, reconnect after a disconnect | Working |
| Password and private key authentication (passphrase supported) | Working |
| Keyboard-interactive authentication | Working (answered with the session's password) |
| Jump hosts: routes through other saved sessions, each hop verified | Working, tested against a real sshd |
| Terminal palettes: follow the interface, or always dark or light | Working |
| Local port forwarding, brought up with its session | Working, tested against a real sshd |
| Remote files: browse (tree or list), create, rename, delete, edit text, upload and download | Working, tested against a real sshd |
| Dynamic and remote forwarding, an agent interface, batch execution | Not implemented |

The application states the same thing on its own first screen: the capability list is read from the
descriptors, which only claim what is implemented.

## Building it

Requirements: Rust (pinned by `rust-toolchain.toml`), Node 22 or newer, pnpm 11,
and on Windows the MSVC toolchain with WebView2 (present on Windows 10 and later).

```bash
pnpm install
pnpm dev:tauri        # the desktop application
pnpm dev              # the interface alone, in a browser, without a backend
```

```bash
pnpm check            # every gate: descriptors, i18n, versions, provenance, guards, lint, types, tests, Rust
pnpm test             # interface tests
cargo test --workspace # Rust tests
pnpm build             # the interface bundle
pnpm icons             # redraw and regenerate the application icons

make lab-up            # a real sshd to connect to, in Docker (127.0.0.1:2222)
make lab-verify        # the live tests: connect, verify a host key, run a command, read the output
make lab-down          # stop it
```

`make` targets are thin wrappers around these commands for people who prefer them.

## How the project is put together

| Path | What lives there |
|---|---|
| `apps/desktop/` | The Vue 3 interface used by the desktop application |
| `crates/` | Rust: the error contract, wire types, and (as they land) the vault, SSH, terminal, SFTP and policy layers |
| `src-tauri/` | The desktop shell: window, command surface, Tauri runtime |
| `descriptors/` | The single source of truth for everything Rust and TypeScript must agree on |
| `packages/` | Cross-layer tests, and later the command line and agent interfaces |
| `scripts/` | Generation, verification and release tooling |
| `docs/` | Architecture, security model, conventions |

Two rules explain most of the layout:

1. **The Rust core may not depend on Tauri.** Hosts (the desktop shell today, a command line
   and an agent interface later) depend on the core, never the other way round. A guard test
   enforces this, so the operations stay callable from something that is not a window.
2. **Anything both languages must agree on is declared once.** Protocols, authentication
   methods and capabilities live in `descriptors/**/*.yaml`; a generator produces the Rust
   enums and the TypeScript types, and `pnpm check:descriptors` fails when they drift apart.

`docs/architecture.md` describes the layers, `docs/security-model.md` the invariants the
code is held to, and `docs/descriptors.md` how to add a protocol.

## Contributing

Bug reports, security reports and contributions are welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md). Two things to know before
opening a pull request: the invariants in `docs/security-model.md` are not negotiable without
a discussion, and every user-visible message needs a key in both locales.

## License

Apache-2.0 — see [LICENSE](LICENSE).
