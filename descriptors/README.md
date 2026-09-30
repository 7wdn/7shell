# Descriptors — the single source of truth

Everything that both Rust and TypeScript must agree on is declared here as YAML and generated from it:

```
descriptors/**/*.yaml
      │  scripts/sync-descriptors.mjs
      ├──────────────────────────► descriptors/descriptors.manifest.json   (committed)
      └──────────────────────────► apps/desktop/src/types/generated/*.ts   (committed)

descriptors/descriptors.manifest.json
      │  crates/sevenshell-types/build.rs  (reads JSON only — no YAML parser in the build graph)
      └──────────────────────────► Protocol / AuthMethod / Capability enums + capability tables
```

Rules:

1. **Never edit generated files by hand.** Edit the YAML, run `pnpm generate:descriptors`, commit both.
   `pnpm check:descriptors` fails when the YAML and the generated files disagree; CI runs it.
2. **Add a capability together with its implementation and tests.** A capability means "this product supports
   this for that protocol", not "the protocol allows it". It is consumed by the UI (which panes to show),
   by the policy layer (which actions are permitted) and later by agent tooling (which tools to expose).
3. **Labels are message keys, never literal text.** `labelKey` values are verified against the locale files
   by `pnpm check:i18n`; a missing key fails the check instead of silently falling back to English.

## Schema

Common fields for every descriptor file:

| Field | Meaning |
|---|---|
| `schemaVersion` | Must be `1`. Bumped only with a migration of the generator. |
| `order` | Stable display order. Unique per kind; gaps are intentional so entries can be inserted. |
| `id` | Stable machine identifier, used in the wire format, the CLI and settings files. Never reused. |
| `rustVariant` | The generated Rust enum variant name. Must be unique and PascalCase. |
| `labelKey` | Message key for the human-readable name. |

`protocols/*.yaml` adds: `defaultPort`, `runtime` (`native` | `sidecar` | `external`), `transport`,
`supportLevel` (`connect` | `browse` | `operate`), `capabilities`.
`auth-methods/*.yaml` adds: `capabilities`, `platforms` (optional; omitted means every platform).

`capabilities.yaml` is the vocabulary: every capability listed there has a `key`, `order`, `group`
(`terminal` | `transfer` | `network` | `session` | `auth` | `trust`) and a `labelKey`. A capability used by a
protocol or an auth method must exist in this catalog — unknown keys fail generation.

A capability that nothing claims yet must say so with `planned: true`, and a `planned` capability may not be
claimed by anything. That keeps the vocabulary from silently drifting away from what the product does: either
a capability is real and something supports it, or it is explicitly marked as not implemented yet.

`labelKey` values must be namespaced (`protocol.*`, `auth.*`, `capability.*`) and end with `.label`.
