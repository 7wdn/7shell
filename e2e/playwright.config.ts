/**
 * End-to-end tests against the running application.
 *
 * They do not launch a browser: they attach to the one the application already has. On Windows that
 * is WebView2, which is Chromium, so the DevTools protocol reaches it — the window is a real
 * client, but almost everything inside it is a web page and can be driven like one.
 *
 * `make app-e2e` starts the application against a temporary data directory, waits for the
 * debugging endpoint and runs these.
 */
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "@playwright/test";

/**
 * Where a failure's own material goes — the page as it was, the call that timed out.
 *
 * Named rather than defaulted: the runner's working directory is the repository root, so the
 * default would scatter `test-results/` beside the sources instead of leaving it with the rest of
 * the end-to-end output that `.gitignore` already accounts for.
 */
const OUTPUT_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "test-results");

export default defineConfig({
  testDir: ".",
  outputDir: OUTPUT_DIR,
  // The whole run is one window and one vault, so a second worker would fight the first.
  workers: 1,
  fullyParallel: false,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  use: {
    // Off unless asked for: a trace is worth megabytes per case, and the reason to ask for one is a
    // case that hung — which is exactly the failure a timeout reports worst. `SEVENSHELL_E2E_TRACE=1`
    // leaves `e2e/test-results/<case>/trace.zip` behind, and its `trace.trace` lists every action with
    // the one that never finished at the end.
    trace: process.env.SEVENSHELL_E2E_TRACE === "1" ? "on" : "off",
    screenshot: "off",
  },
});
