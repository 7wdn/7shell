/**
 * Contracts between the Rust backend and the interface.
 *
 * The two halves of the wire format are written by hand — there is no code generation
 * across the IPC boundary, because generated command types cost more than they save at
 * this size. What replaces the generator is this file: it reads the Rust sources and
 * fails when the two sides disagree about a code, a message key or a field name.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ERROR_CATALOG } from "@/lib/backend/errorCodes";
import type { ErrorEnvelope, ErrorOrigin } from "@/lib/backend/errors";
import type { AppInfo } from "@/types/app";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function read(relativePath: string): string {
  return readFileSync(path.join(REPO_ROOT, relativePath), "utf8");
}

function camelCase(name: string): string {
  return name.replace(/_([a-z0-9])/g, (_, letter: string) => letter.toUpperCase());
}

function sorted(names: readonly string[]): string[] {
  return [...names].sort();
}

/** `Variant => "CODE", "message.key";` pairs from the Rust catalog macro. */
function rustErrorCatalog(source: string): Map<string, string> {
  const entries = new Map<string, string>();
  for (const match of source.matchAll(/=>\s*"([A-Z0-9-]+)"\s*,\s*"([A-Za-z0-9_.]+)"/g)) {
    entries.set(match[1] as string, match[2] as string);
  }
  return entries;
}

/** Field names of a Rust struct, in declaration order, attributes and visibility ignored. */
function rustStructFields(source: string, structName: string): string[] {
  const body = new RegExp(`(?:pub )?struct ${structName} \\{([\\s\\S]*?)\\n\\}`).exec(source)?.[1] ?? "";
  return [...body.matchAll(/^\s{4}(?:pub )?([a-z_][a-z0-9_]*)\s*:/gm)].map((match) => match[1] as string);
}

/** Variant names of a Rust enum, lower-cased to match how they are serialized. */
function rustEnumVariants(source: string, enumName: string): string[] {
  const body = new RegExp(`pub enum ${enumName} \\{([\\s\\S]*?)\\n\\}`).exec(source)?.[1] ?? "";
  return [...body.matchAll(/^\s{4}([A-Z][A-Za-z0-9]*),/gm)].map((match) => (match[1] as string).toLowerCase());
}

/** String members of a TypeScript union type. */
function tsStringUnion(source: string, typeName: string): string[] {
  const body = new RegExp(`export type ${typeName} =([\\s\\S]*?);`).exec(source)?.[1] ?? "";
  return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1] as string);
}

/** `readonly name` members of a TypeScript interface. */
function interfaceFields(source: string, interfaceName: string): string[] {
  const body = new RegExp(`export interface ${interfaceName} \\{([\\s\\S]*?)\\n\\}`).exec(source)?.[1] ?? "";
  return [...body.matchAll(/readonly ([A-Za-z_][A-Za-z0-9_]*)\??:/g)].map((match) => match[1] as string);
}

describe("error catalog", () => {
  const rust = rustErrorCatalog(read("crates/sevenshell-error/src/code.rs"));
  const typescript = new Map(Object.entries(ERROR_CATALOG));

  it("is read from both sides", () => {
    expect(rust.size).toBeGreaterThan(0);
    expect(typescript.size).toBe(rust.size);
  });

  it("has every Rust code with the same message key in the interface", () => {
    for (const [code, messageKey] of rust) {
      expect(typescript.get(code), `missing or different in the interface: ${code}`).toBe(messageKey);
    }
  });

  it("has no interface code that Rust does not define", () => {
    for (const code of typescript.keys()) {
      expect(rust.has(code), `unknown to Rust: ${code}`).toBe(true);
    }
  });
});

describe("settings types", () => {
  const rustSettings = read("crates/sevenshell-types/src/settings.rs");
  const interfaceTypes = read("apps/desktop/src/types/app.ts");

  it("declares the same AppSettings fields on both sides", () => {
    const rustFields = rustStructFields(rustSettings, "AppSettings").map(camelCase);

    expect(rustFields.length).toBeGreaterThan(0);
    expect(sorted(rustFields)).toEqual(sorted(interfaceFields(interfaceTypes, "AppSettings")));
  });

  it("declares the same theme modes on both sides", () => {
    const rustModes = rustEnumVariants(rustSettings, "ThemeMode");
    const typescriptModes = tsStringUnion(interfaceTypes, "ThemeMode");

    expect(sorted(rustModes)).toEqual(["dark", "light", "system"]);
    expect(sorted(typescriptModes)).toEqual(sorted(rustModes));
  });
});

describe("wire types", () => {
  it("declares the same AppInfo fields on both sides", () => {
    const rustFields = rustStructFields(read("crates/sevenshell-types/src/app_info.rs"), "AppInfo").map(camelCase);

    expect(rustFields.length).toBeGreaterThan(0);
    expect(sorted(rustFields)).toEqual(sorted(interfaceFields(read("apps/desktop/src/types/app.ts"), "AppInfo")));
  });

  it("declares the same session fields on both sides", () => {
    const rust = read("crates/sevenshell-types/src/session.rs");
    const typescript = read("apps/desktop/src/types/session.ts");

    for (const name of [
      "Session",
      "SessionAuth",
      "KeyInfo",
      "TerminalSettings",
      "SessionGroup",
      "KnownHostEntry",
      "PresentedHostKey",
    ]) {
      const rustFields = rustStructFields(rust, name).map(camelCase);

      expect(rustFields.length, `${name} has no fields, so the comparison proves nothing`).toBeGreaterThan(0);
      expect(sorted(rustFields), `${name} differs between Rust and the interface`).toEqual(
        sorted(interfaceFields(typescript, name)),
      );
    }
  });

  it("has no field a secret could travel out in", () => {
    // Invariant 1: a password travels inwards only. What the DTOs carry is a boolean saying one
    // is stored — so a field named like a secret, other than those two booleans, is a bug.
    const rust = read("crates/sevenshell-types/src/session.rs");
    const fields = [
      "Session",
      "SessionAuth",
      "KeyInfo",
      "TerminalSettings",
      "SessionGroup",
      "KnownHostEntry",
      "PresentedHostKey",
    ].flatMap((name) => rustStructFields(rust, name).map(camelCase));

    expect(fields.length).toBeGreaterThan(0);
    expect(fields.filter((field) => /secret|password|passphrase|material/i.test(field) && !field.startsWith("has"))).toEqual(
      [],
    );
    // The one thing that travels inwards is named for what it is, and has no way back.
    expect(sorted(rustStructFields(rust, "SecretInput"))).toEqual(["value"]);
  });

  it("declares the same vault states on both sides", () => {
    const rustStates = rustEnumVariants(read("crates/sevenshell-types/src/session.rs"), "VaultState");

    expect(sorted(rustStates)).toEqual(["locked", "missing", "unlocked"]);
    expect(sorted(tsStringUnion(read("apps/desktop/src/types/session.ts"), "VaultState"))).toEqual(sorted(rustStates));
  });

  it("declares the same envelope fields on both sides", () => {
    // The envelope is a newtype over `ShellErrorFields` so the error stays small; the JSON
    // shape is still flat, which `crates/sevenshell-error/tests/wire_format.rs` pins.
    const rustFields = rustStructFields(read("crates/sevenshell-error/src/envelope.rs"), "ShellErrorFields").map(camelCase);

    expect(rustFields.length).toBeGreaterThan(0);
    expect(sorted(rustFields)).toEqual(sorted(interfaceFields(read("apps/desktop/src/lib/backend/errors.ts"), "ErrorEnvelope")));
  });

  it("declares the same origin fields on both sides", () => {
    const rustFields = rustStructFields(read("crates/sevenshell-error/src/envelope.rs"), "Origin").map(camelCase);

    expect(sorted(rustFields)).toEqual(sorted(interfaceFields(read("apps/desktop/src/lib/backend/errors.ts"), "ErrorOrigin")));
  });

  it("keeps the compiled interfaces aligned with the Rust structs", () => {
    // These assignments exist so `tsc` fails if a field is renamed in one language only.
    const envelope: ErrorEnvelope = {
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      messageParams: {},
      origin: { subsystem: "app" },
      outcome: "unknown",
    };
    const origin: ErrorOrigin = { subsystem: "ssh", adapter: "russh", peer: "host.example:22" };
    const info: AppInfo = {
      name: "7Shell",
      version: "0.1.0",
      os: "windows",
      arch: "x86_64",
      debugBuild: true,
      protocolCount: 1,
      descriptorSchemaVersion: 1,
    };

    expect(envelope.outcome).toBe("unknown");
    expect(origin.subsystem).toBe("ssh");
    expect(info.name).toBe("7Shell");
  });
});
