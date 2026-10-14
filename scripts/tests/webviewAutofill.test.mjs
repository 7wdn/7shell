// The platform's general autofill, turned off at the window.
//
// WebView2 keeps a suggestion history for ordinary text fields — the box headed "Saved info" in the
// platform's own words — and offers it back under any field it recognizes: a session's user, host
// or port. The history lives in the *webview's* data folder, which is not the settings directory
// this repository hands tests, so every end-to-end run of these months of work was also feeding it
// real entries: `session-user` had thousands of them, and the person's window offered `labuser`
// back to them. The relevant reading, and the reason the switch is here rather than a per-field
// `autocomplete="off"` (which WebView2's own documentation says its Suggestions flyout "may not
// honor"), is the `generalAutofillEnabled` key in `src-tauri/tauri.conf.json`: it maps through
// tauri-utils -> tauri -> wry to `SetIsGeneralAutofillEnabled(false)` on every platform that has
// the feature, and on the rest it performs no operation.
//
// The failure this guards against is silent in both directions: deleting the key turns the history
// and the flyout back on with nothing going red, and a config rewrite ("tidy the window keys") is
// exactly the kind of edit that drops it. It is asserted at the file because that is where the
// decision lives; what an installed build does with it is the platform's to honor.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const CONFIG = path.join(REPO_ROOT, "src-tauri", "tauri.conf.json");

test("the webview's general autofill is off in the window config", () => {
  const windows = JSON.parse(readFileSync(CONFIG, "utf8")).app?.windows ?? [];
  const main = windows.find((window) => window.label === "main");

  assert.ok(main, "the main window is declared in the config");
  assert.equal(
    main.generalAutofillEnabled,
    false,
    "without this, the platform collects field history into the webview's data folder and offers it back under every field — the tests' own values among them",
  );
});
