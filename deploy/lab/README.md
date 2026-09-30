# SSH test lab

A real `sshd` to test against, because everything interesting about an SSH client only shows up
against a real server: host key verification, authentication, a pty, resize, and what a server says
when it closes a channel.

```bash
make lab-up        # start the server (needs Docker)
make lab-verify    # run the live tests against it
make lab-down      # stop it
```

The server listens on **127.0.0.1:2222** only — a loopback binding, so nothing on the network can
reach it — and its credentials are deliberately trivial (`labuser` / `labpassword`). It is a test
fixture, not something to expose.

## What the recipes will cover

| Recipe | What it is for |
|---|---|
| `openssh` | The baseline: password authentication, an ed25519 host key, a shell. |
| `openssh-kbdint` | Keyboard-interactive, which many servers offer instead of `password`. |
| `openssh-pubkey` | Public key authentication, including a key with a passphrase. |
| `openssh-multikey` | Several host key algorithms, to exercise algorithm negotiation. |
| `openssh-flaky` | Delays and half-open connections, for timeout and reconnect behaviour. |

Only `openssh` exists so far; the rest arrive with the features that need them, rather than sitting
around as files nobody runs.

## Running the tests by hand

```bash
SEVENSHELL_TEST_HOST=127.0.0.1 \
SEVENSHELL_TEST_PORT=2222 \
SEVENSHELL_TEST_USER=labuser \
SEVENSHELL_TEST_PASSWORD=labpassword \
cargo test -p sevenshell-ssh -p sevenshell-core -- --ignored
```

Tests that need the lab are `#[ignore]`d and read those variables. Without them they report that
the lab is not configured and pass, so an ordinary `cargo test` stays offline and fast.
