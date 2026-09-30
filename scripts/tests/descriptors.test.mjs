// Descriptor generation, from the generator's own side.
//
// `check:descriptors` proves the committed files match this generator's output; these
// tests prove the generator agrees with the Rust side about naming and ordering, which is
// the part a byte comparison cannot see.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import {
  GENERATED_TS_DIR,
  MANIFEST_PATH,
  loadDescriptors,
  pascalCase,
  serializeManifest,
} from "../lib/descriptors.mjs";

test("wire identifiers map to the Rust variants the generator expects", () => {
  assert.equal(pascalCase("password"), "Password");
  assert.equal(pascalCase("publicKey"), "PublicKey");
  assert.equal(pascalCase("keyboardInteractive"), "KeyboardInteractive");
  assert.equal(pascalCase("agentForwarding"), "AgentForwarding");
  assert.equal(pascalCase("agent-forwarding"), "AgentForwarding");
  assert.equal(pascalCase("key_material"), "KeyMaterial");
});

test("the committed manifest is exactly what the descriptors describe", async () => {
  const { manifest } = await loadDescriptors();
  assert.equal(readFileSync(MANIFEST_PATH, "utf8"), serializeManifest(manifest));
});

test("the generated TypeScript covers every descriptor", async () => {
  const { capabilities, protocols, authMethods } = await loadDescriptors();
  const typescript = readFileSync(path.join(GENERATED_TS_DIR, "protocols.ts"), "utf8");

  for (const protocol of protocols) {
    assert.ok(typescript.includes(`id: "${protocol.id}",`), `${protocol.id} is missing from the generated protocols`);
  }
  for (const method of authMethods) {
    assert.ok(typescript.includes(`id: "${method.id}",`), `${method.id} is missing from the generated auth methods`);
  }
  for (const capability of capabilities) {
    assert.ok(
      typescript.includes(`key: "${capability.key}",`),
      `${capability.key} is missing from the generated capabilities`,
    );
  }
});

test("descriptors are uniquely identified and ordered", async () => {
  const { capabilities, protocols, authMethods } = await loadDescriptors();

  for (const group of [protocols, authMethods, capabilities]) {
    const ids = group.map((entry) => entry.id ?? entry.key);
    assert.equal(new Set(ids).size, ids.length, "identifiers must be unique");
  }

  for (const group of [protocols, authMethods, capabilities]) {
    const orders = group.map((entry) => entry.order);
    assert.deepEqual(
      orders,
      [...orders].sort((left, right) => left - right),
      "descriptors must be ordered",
    );
  }
});
