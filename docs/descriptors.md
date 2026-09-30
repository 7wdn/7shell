# Descriptors

`descriptors/` holds the one place where protocols, authentication methods and the capability
vocabulary are declared. Rust and TypeScript are both generated from it, so the two languages
cannot disagree about what the product supports.

The rules for editing the YAML — including the `planned` flag and what `labelKey` must look like
— are in [descriptors/README.md](../descriptors/README.md), next to the files themselves. This
document is about the pipeline around them.

## The pipeline

```
descriptors/**/*.yaml
   │
   │  pnpm generate:descriptors      (scripts/sync-descriptors.mjs)
   ├───────────────────────────────►  descriptors/descriptors.manifest.json      committed
   └───────────────────────────────►  apps/desktop/src/types/generated/protocols.ts  committed

descriptors/descriptors.manifest.json
   │
   └───────────────────────────────►  crates/sevenshell-types/build.rs
                                      → Protocol / AuthMethod / Capability enums,
                                        capability and platform tables
```

Four decisions are deliberate:

**The manifest is committed.** `build.rs` reads JSON, so the Rust build graph needs no YAML
parser, and `cargo build` works in a fresh clone without running anything first. The price is that
someone can edit the YAML and forget to regenerate — which is exactly what
`pnpm check:descriptors` exists to catch. It regenerates in memory and compares byte for byte;
CI runs it, and so does `pnpm check`.

**Naming is derived, not declared twice.** The Rust enum variant for a capability or a group is
computed from its wire identifier (`agentForwarding` → `AgentForwarding`), so there is no second
field to keep in step. Since the JavaScript generator and the Rust generator must agree on this
rule, `scripts/tests/descriptors.test.mjs` pins the examples.

**Labels are keys, never text.** A `labelKey` such as `capability.sftp.label` is verified against
every locale by `pnpm check:i18n`. A missing translation fails the check instead of quietly
falling back to English, because a half-translated interface is a bug report waiting to happen.

**Capabilities describe the product, not the protocol.** A capability means "7Shell supports this
for that protocol", and it is added by the change that implements and tests it. A protocol that
declares a capability the interface cannot honour would offer a button that does nothing. When a
capability is real in the protocol but not yet implemented here, it sits in the catalog with
`planned: true` and no descriptor claims it, which is how the YAML stays honest about the gap
between what SSH can do and what this project can do so far.

## Adding a protocol

1. Add `descriptors/protocols/<id>.yaml` with `schemaVersion`, a unique `order`, an `id`,
   a Rust `rustVariant`, a `labelKey` under `protocol.`, `defaultPort`, `runtime`, `transport`,
   `supportLevel` and the capabilities you are committing to.
2. Add any new capabilities to `descriptors/capabilities.yaml` first.
3. Run `pnpm generate:descriptors` and commit both generated files.
4. Add the labels to every locale under `apps/desktop/src/i18n/locales/<language>/protocol.json`.
5. Run `pnpm check` — the descriptor tests, the i18n check and the Rust tests all exercise the
   result before any implementation code exists.

## Adding an authentication method

Same shape, in `descriptors/auth-methods/`, with `capabilities` describing what the method needs
(`secret`, `keyMaterial`, `interactive`, `platformAgent`) and an optional `platforms` list. An
empty `platforms` means every platform; a method that only exists on one platform currently
declares it explicitly, which is why Pageant is not offered on Linux.

## What the generated tables are used for

- The interface: which panes and fields exist (`protocolSupports` in the generated TypeScript).
- The core and policy layers: what may be attempted for a connection at all.
- The connection editor (planned): the default port, and which fields an auth method needs.
- The agent interface (planned): which tools to expose. A tool for something the connection
  cannot do is not registered, rather than registered and failing at call time.
