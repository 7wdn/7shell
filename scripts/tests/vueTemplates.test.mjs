// Every component a template names has to be imported.
//
// Vue does not fail on an unresolved tag: it renders a custom element that keeps its place, answers
// a click and draws nothing. A control that is invisible but present is the worst kind of bug — it
// ships, it is in the DOM, and only a person looking at the screen can find it. This guard is what
// makes the compiler's silence loud.
//
// What it checks: every PascalCase tag in a `.vue` template is either imported by that file or is a
// built-in it cannot be (teleports, transitions, slots, suspense). Native elements are lowercase and
// never match.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const SOURCE_ROOT = path.join(REPO_ROOT, "apps", "desktop", "src");

/** Vue's own components, which no file imports. */
const BUILT_IN = new Set(["Transition", "TransitionGroup", "KeepAlive", "Teleport", "Suspense", "Component"]);

function templates() {
  const out = [];
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith(".vue")) {
        out.push(full);
      }
    }
  };
  walk(SOURCE_ROOT);
  return out;
}

/** The names a file brings into scope: default imports, named imports, and multiple per line. */
function imports(source) {
  const names = new Set();
  for (const match of source.matchAll(/import\s+([A-Z][A-Za-z0-9]*)\s*(?:,|from)/g)) {
    names.add(match[1]);
  }
  for (const block of source.matchAll(/import\s*(?:type\s*)?\{([^}]*)\}\s*from/g)) {
    for (const part of block[1].split(",")) {
      const name = part
        .trim()
        .split(/\s+as\s+/)
        .pop()
        ?.trim();
      if (name && /^[A-Z]/.test(name)) {
        names.add(name);
      }
    }
  }
  return names;
}

test("every component a template names is imported by that file", () => {
  const offenders = [];

  for (const file of templates()) {
    const source = readFileSync(file, "utf8");
    const start = source.indexOf("<template>");
    if (start < 0) {
      continue;
    }
    const template = source.slice(start);
    const available = imports(source);

    for (const tag of new Set([...template.matchAll(/<([A-Z][A-Za-z0-9]*)/g)].map((match) => match[1]))) {
      if (!available.has(tag) && !BUILT_IN.has(tag)) {
        offenders.push(`${path.relative(REPO_ROOT, file)}: <${tag}>`);
      }
    }
  }

  assert.deepEqual(offenders, [], `a tag with no import renders as nothing at all: ${offenders.join(", ")}`);
});
