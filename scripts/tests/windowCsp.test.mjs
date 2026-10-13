// The window's style policy, which the panel of the terminal depends on.
//
// xterm.js does not ship its colours, its font or its grid in a stylesheet: it writes one at
// runtime, into a `<style>` element it creates itself. That stylesheet carries everything the pane
// draws with — the family and the size of the text, the foreground and the sixteen colours, the
// cursor, the selection, and the height of a row.
//
// A content security policy that carries a nonce in `style-src` blocks it. The element stays in the
// document with its text intact and is simply never applied, so the terminal falls back to what it
// inherits: the interface's own proportional font at the interface's own size. The symptom is a
// terminal that ignores "font size" and "terminal font" completely — and it appears *only* in an
// installed build, because Tauri puts the nonce there and `tauri dev` never sees a policy at all
// (the page comes from the dev server, which sends no CSP header). That is how it shipped in
// 0.1.0 and 0.1.1 while every test in this repository was green.
//
// So the policy is asserted here rather than left to the config: `style-src` has to stay a
// directive that allows an inline stylesheet, which means Tauri must not be allowed to rewrite it.
// The whole story, and what it means for verification, is in `docs/security-model.md`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const CONFIG = path.join(REPO_ROOT, "src-tauri", "tauri.conf.json");

/** The directives of one policy, as a map from name to its sources. */
function directives(policy) {
  const parsed = new Map();
  for (const group of policy.split(";")) {
    const [name, ...sources] = group.trim().split(/\s+/).filter(Boolean);
    if (name) {
      parsed.set(name.toLowerCase(), sources);
    }
  }
  return parsed;
}

function config() {
  return JSON.parse(readFileSync(CONFIG, "utf8"));
}

test("the shipped policy lets a library write its own stylesheet", () => {
  const security = config().app?.security;
  const style = directives(security?.csp ?? "").get("style-src");

  assert.ok(style, "the shipped policy has no style-src at all");
  assert.ok(
    style.includes("'unsafe-inline'"),
    "style-src must allow an inline stylesheet: the terminal's own font, size and colours are one",
  );
  assert.ok(
    !style.some((source) => source.startsWith("'nonce-") || source.startsWith("'sha")),
    "a nonce or hash in style-src makes 'unsafe-inline' inert, which silently blocks the terminal's stylesheet",
  );
});

test("Tauri is told to leave style-src alone, because it would put a nonce there", () => {
  const security = config().app?.security;
  const disabled = security?.dangerousDisableAssetCspModification;

  assert.deepEqual(
    disabled,
    ["style-src"],
    "without this, Tauri adds a nonce to style-src and the terminal quietly ignores its own settings — in installed builds only",
  );
});
