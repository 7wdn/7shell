// The webview's own browser keys, turned off where the window is prepared.
//
// WebView2 answers a handful of keys before the interface ever sees them: Ctrl+F and F3 open its
// find bar over whatever is on screen — over the shell's chrome, and beside the terminal's own find
// when one is open — F5 and Ctrl+R reload the page and take the open tabs with them, and Ctrl+P,
// the zoom keys and F12 come along. The switch that turns them off is
// `SetAreBrowserAcceleratorKeysEnabled(false)` in `window_chrome::silence_browser_keys`, made
// through the controller `with_webview` hands over.
//
// The end-to-end run cannot hold this down: it drives the window through the debugging protocol,
// which never passes the browser's own accelerator handling, so the find bar this guards against
// does not appear in a test whether the switch is there or not — deleting it would leave every run
// green while a real Ctrl+F kept opening the bar over the shell. So the guard is at the source: the
// switch and its call must be there, and removing either fails here instead of shipping silently.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

test("the webview's browser keys are turned off in the window's preparation", () => {
  const chrome = readFileSync(path.join(REPO_ROOT, "src-tauri", "src", "window_chrome.rs"), "utf8");
  assert.match(
    chrome,
    /\.SetAreBrowserAcceleratorKeysEnabled\(false\)/,
    "without this, WebView2 answers Ctrl+F with its own find bar over whatever is on screen",
  );

  const lib = readFileSync(path.join(REPO_ROOT, "src-tauri", "src", "lib.rs"), "utf8");
  assert.match(
    lib,
    /^[ \t]*window_chrome::silence_browser_keys\(app\.handle\(\)\);/m,
    "the switch is made by silence_browser_keys, and a function nobody calls turns nothing off — a commented-out call is a call that does not run",
  );
});
