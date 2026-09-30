# app-tests

Tests that belong to no single layer: they compare what the interface believes with what
Rust actually declares. `vitest` runs them from the repository root, so they may import
from `apps/desktop/src` through the `@` alias.

Contract tests here deliberately read the Rust sources as text. That is blunt, but it is the
only way to fail when one side renames a field and the other does not — a real risk, because
the two halves are written by hand on purpose (no code generation across the IPC boundary).

| Test | What it protects |
|---|---|
| `contracts.test.ts` | The error catalog and the wire DTOs match the Rust definitions exactly. |
