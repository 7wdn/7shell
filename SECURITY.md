# Security policy

7Shell handles credentials, private keys and live access to servers. Security reports are
taken seriously and answered as quickly as they can be.

## Reporting a vulnerability

Open a **private security advisory** on the project's repository
(*Security → Report a vulnerability*). Please do not open a public issue for a suspected
vulnerability.

Include what you did, what happened, what you expected, and the version or commit you tested.
A proof of concept helps enormously. If you are not sure whether something counts, report it
anyway — "it probably does not matter" is how most of these get missed.

What you can expect: an acknowledgement, an assessment of severity and reach, and a fix or a
reasoned explanation. Credit in the release notes if you would like it.

## Supported versions

No version has been released yet, so nothing is supported for production use. Once releases
begin, fixes will target the latest minor release.

## The security model in one screen

The full model, including the threat model and how each rule is tested, is in
[docs/security-model.md](docs/security-model.md). The invariants that cannot be traded away
for convenience:

| # | Invariant |
|---|---|
| 1 | A secret never leaves the process: passwords and key material only ever travel *in*. |
| 2 | No credential is sent to a server before its host key has been approved. |
| 3 | An unknown host key is never accepted automatically, and a changed host key is a hard stop. |
| 4 | A decryption failure is indistinguishable from any other decryption failure. |
| 5 | The vault is written atomically; an interrupted write leaves the previous file usable. |
| 6 | The interface never names a local path; it receives tokens for files the user picked. |
| 7 | Every privileged or destructive action has a policy decision and an audit event. |

## What is in scope

The desktop application, the Rust crates, the descriptors, the build and release tooling, and
the supply chain of the dependencies themselves. Out of scope: attacks that require an already
compromised machine or a malicious server you deliberately connected to — although a server
that can make the client leak something *is* in scope, and is interesting.
