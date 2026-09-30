// The interface talks to the backend through one module.
//
// Everything else imports the facade, so a second transport can be added by editing one
// file instead of hunting every call site. This guard is what makes that promise true.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const SOURCE_ROOT = path.join(REPO_ROOT, "apps", "desktop", "src");
const TRANSPORT_MODULE = "lib/backend/tauri.ts";
const FORBIDDEN = "@tauri-apps/api";

function sourceFiles(directory) {
  const out = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      out.push(...sourceFiles(full));
      continue;
    }
    if (/\.(ts|vue)$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

test("only the transport module imports the Tauri API", () => {
  const offenders = [];

  for (const file of sourceFiles(SOURCE_ROOT)) {
    const relative = path.relative(SOURCE_ROOT, file).split(path.sep).join("/");
    if (relative === TRANSPORT_MODULE) {
      continue;
    }
    if (readFileSync(file, "utf8").includes(FORBIDDEN)) {
      offenders.push(relative);
    }
  }

  assert.deepEqual(offenders, [], `import the backend facade instead of ${FORBIDDEN}: ${offenders.join(", ")}`);
});

test("every backend call is routed through the facade", () => {
  const facade = readFileSync(path.join(SOURCE_ROOT, "lib", "backend", "api.ts"), "utf8");

  assert.ok(facade.includes("isDesktopRuntime"), "the facade must answer calls with no transport in a defined way");
  assert.ok(facade.includes("normalizeShellError"), "the facade must normalize whatever the backend rejects with");
});
