# Test lab

Real servers to test against, because everything interesting about a terminal client only shows up
against one: host key verification, authentication, a pty, resize, and what a server says when it
closes a channel.

```bash
make lab-up        # start the servers (needs Docker)
make lab-verify    # run the live tests against them
make lab-down      # stop them
```

The same thing runs in CI (`.github/workflows/lab.yml`): nightly, on demand, and on any pull request
that touches the crates the live tests cover. Locally is for writing them; CI is for noticing when
they stop passing.

The server listens on **127.0.0.1:2222** only — a loopback binding, so nothing on the network can
reach it — and its credentials are deliberately trivial (`labuser` / `labpassword`). It is a test
fixture, not something to expose.

The Telnet console listens on **127.0.0.1:23230** (busybox `telnetd` from `alpine`), also loopback
only. The host port is not 23 because this machine's Windows reserves 1779-2978 for Hyper-V.

## The container and cluster labs

`openssh/compose.yaml` also starts a `docker:dind` service (`sevenshell-lab-docker`). The sshd
container installs the Docker CLI at start-up (`init.d/20-docker-cli.sh`) and reaches that daemon
over the compose network, which is what lets the container-over-SSH live tests run against a real
engine without giving a Linux container access to a Windows named pipe. The daemon publishes no
host port.

`kubernetes/compose.yaml` is a one-node k3s for the cluster live tests. Copy its kubeconfig out
with `docker exec sevenshell-lab-k3s cat /etc/rancher/k3s/k3s.yaml > kubeconfig.yaml`; k3s writes
an SEC1 EC client key and the client normalizes it before building the TLS identity.

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

SEVENSHELL_TEST_TELNET_HOST=127.0.0.1 \
SEVENSHELL_TEST_TELNET_PORT=23230 \
cargo test -p sevenshell-term --test live_telnet -- --ignored
```

Tests that need the lab are `#[ignore]`d and read those variables. Without them they report that
the lab is not configured and pass, so an ordinary `cargo test` stays offline and fast.
