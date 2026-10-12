# Troubleshooting

## Read the error first

Every failure the application reports is one shape: a stable `code`, a message key with the values
that belong in it, the subsystem it came from, and — for anything that touched a credential — an
`outcome` saying whether anything was sent. The interface draws the message in your language and the
code beneath it in monospace. The message is for reading; the code is for searching, for comparing
two machines, and for a bug report.

Codes look like `SSH-HOSTKEY-MISMATCH`, `VAULT-UNLOCK-FAILED`, `SFTP-TRANSFER-FAILED`. The catalog
lives in `apps/desktop/src/lib/backend/errorCodes.ts`, mirroring the Rust catalog in
`crates/sevenshell-error/src/code.rs`, and a contract test fails when the two disagree in either
direction.

Two properties are worth knowing when you look at one:

- **`outcome` distinguishes "nothing was sent" from "something was sent".** After an authentication
  failure that is the first question anybody has, and the interface surfaces the answer rather than
  leaving it to be read out of the code.
- **Detail is scrubbed before it becomes a message.** Private key blocks, credential assignments and
  bearer tokens are removed on the way into an envelope, parameter values whose key looks like a
  secret are masked, and detail is truncated at 64 KiB. That is what makes an error safe to paste —
  but read it once before you post it anyway.

## A host key has changed

**Symptom.** The connection stops with "The server is presenting a different host key", showing the
fingerprint presented now beside the one trusted before.

**What it means.** The key recorded for that address and port is not the one being presented. Either
the machine was rebuilt or reinstalled and has a new key, or the connection is being intercepted.
This is deliberately a stop rather than a prompt to click past: the two fingerprints are what a
person needs to decide, and a warning that can be dismissed is one that will be dismissed.

**What to do.** Find out which of the two it is. If you know why the key changed — a reinstall, a
server that was given a new key on purpose — use **Forget the old key and trust this one** on that
same screen. If you do not, cancel. Nothing was sent to that server: the key is compared over a
connection that sends no user name, no password and no key. Settings → Security lists the host keys
this device trusts, with an action that stops trusting one.

## Authentication failed

**Symptom.** `SSH-AUTH-FAILED`: "The server at {host}:{port} rejected these credentials."

**What it means.** The credentials arrived and the server refused them. Common causes: the password
is wrong or out of date; the user name is not the one the server knows; the public key is not in
that account's `authorized_keys`; a key's passphrase is stored wrong; or the server does not accept
that method at all — some servers offer keyboard-interactive and not `password`.

**What to check.**

- The session row says which method it uses and whether a credential is stored: `Password · no
  credential stored` is a session that cannot connect until one is saved.
- For a key, the public half is what goes on the server. Settings → Security → **Keys this device
  holds** shows it (**Show the public key**, then Copy); it is meant to be pasted into
  `authorized_keys` on the server.
- `SSH-KEY-INVALID` — the key file could not be read; if it has a passphrase, check that too.
- `SSH-KEY-UNSUPPORTED` — this build cannot use that kind of key.
- Pageant is listed in the editor on Windows and is not selectable, because this build does not
  implement it yet.

## The host cannot be reached

**Symptom.** `SSH-UNREACHABLE`: "Could not reach {host}:{port}."

**What it means.** The connection never got to a handshake. The name may not resolve, nothing may be
listening on that port, or something between the two machines may be dropping the traffic — a
firewall that *drops* rather than rejects usually looks like a timeout instead.

**What to check.** The address and port saved in the session; whether the name resolves on this
machine; whether the port is open from where you are sitting, with the platform's own tool rather
than the application (`Test-NetConnection host -Port 22` on Windows, `nc -vz host 22` elsewhere);
and the server's firewall, including any allow-list that has to name your address.

## Timeouts, and timeouts through a jump host or a proxy

**Symptoms.** `SSH-TIMEOUT`: "{host}:{port} did not respond within {seconds} seconds." Or
`NET-PROXY-FAILED`: "The proxy could not be reached."

**What it means.** A route is opened hop by hop, in the order the session lists the jump hosts, and
every hop's key is verified before anything is sent through it. So **an error naming a jump host is
about that hop**, not about the destination, and the fix is on that machine or that link.

Things that go wrong in this shape:

- The first hop is unreachable while the destination is fine: the route fails at the first hop, and
  the error names it.
- A hop whose key has never been seen: the decision names *that host*, so the same prompt covers a
  route of any length — trust it, and the next unknown host asks in its turn. Nothing stays
  half-open while you think.
- A hop whose key has changed: a stop, exactly as for a destination.
- A proxy in front of a hop: `NET-PROXY-FAILED` means the proxy itself could not be reached, which is
  a different failure from the far side being down.
- A handshake that is merely slow on a loaded link: the timeout is measured on the connection
  attempt, and a server that needs longer shows up as `SSH-TIMEOUT` rather than as a refusal.

A port forward that cannot listen — a port already taken, most often — does not fail the terminal.
The reason travels back with the result and is shown next to the forwards that did come up, in the
**Tunnels** strip under the terminal.

## A transfer failed

The **Transfers** list is the record, and it is where to look rather than at the progress bar that
stopped. Each row carries its direction, its state — Running, Finished, Failed, Cancelled, Paused —
what it has moved (`{transferred} / {size}`, or `unknown` until the size is known) and, when
something went wrong, the reason with the error's code beneath it. Failures are also announced as a
toast, so a failure in a panel that is not on screen is not silent.

What the states mean for the bytes on disk:

- A transfer writes beside its target — `notes.txt.7shell-part` — and renames it into place only
  after every byte has arrived. A half-written file never wears the name you are checking, on either
  end.
- **Cancelled** removes the part file (somebody said stop). **Failed** keeps it, because the next
  thing you will press is **Try again**, and the part file's length is how much arrived.
- A retry continues from what arrived, but only while the source is unchanged: same size, same
  modification time. A transfer's *first* attempt clears a part file it did not make, because a part
  file left by an earlier run cannot be checked against what it came from.
- A folder goes over as one packed archive, made on the far side by the host's own `tar` and opened
  into the destination folder on arrival. A host without `tar` cannot transfer a folder at all:
  `SFTP-ARCHIVE-FAILED`. Single files are copied as they always were.
- The server's answers name the path: `SFTP-PERMISSION-DENIED`, `SFTP-NOT-FOUND`, `SFTP-EXISTS`. The
  local end has its own codes: `FS-OPEN-FAILED` (the location could not be opened on this system) and
  `FS-GRANT-EXPIRED` (the access to a chosen file ended when the vault was locked).
- If a download folder has been remembered (**Download to** in the transfers list), a name that
  already exists there is replaced without a question — that is what having a folder removes. Set it
  back to **Ask every time** if you want the save dialog's question back.

## Reproduce it against the lab

A real `sshd` in a container is what most of this is tested against, and it is the fastest way to
find out whether a problem is the client or the server:

```bash
make lab-up        # cd deploy/lab/openssh && docker compose up -d
make lab-verify    # the live tests, against 127.0.0.1:2222
make lab-down
```

The account is `labuser` / `labpassword` and the server binds loopback only; it is a test fixture.
The same session in the application is enough to reproduce host key behaviour by hand — including
the hard stop, by replacing the server's host key and connecting again.

On Windows the lab's port is often unavailable: Hyper-V and WSL reserve ranges of ports, so
`docker compose up` fails with "an attempt was made to access a socket in a way forbidden by its
access permissions" rather than "already in use". `netsh interface ipv4 show excludedportrange
protocol=tcp` shows the reserved ranges; publish the lab on a port outside them with a compose
override file (see [Install](install.md#the-ssh-test-lab)), and point `SEVENSHELL_TEST_PORT` at it.
The recipe in the repository stays on 2222 because CI uses it.

## Reporting it

A useful report carries the error's **code** (the monospace line under the message), the platform and
version from Settings → About, what you did just before, and what the far side is (server software
and version, and whether a jump host or proxy is in the route). Include the message text if it is in
your language. Do not include a password, a private key or a full environment dump.
