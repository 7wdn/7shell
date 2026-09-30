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
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  // The whole run is one window and one vault, so a second worker would fight the first.
  workers: 1,
  fullyParallel: false,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  use: {
    trace: "off",
    screenshot: "off",
  },
});
