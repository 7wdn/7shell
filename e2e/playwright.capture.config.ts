/**
 * Runs the screenshot capture against the real window.
 *
 * Its own config rather than the suite's, because the two are asked for by different people. The
 * suite is run on every push and must not need a server; the capture needs the SSH lab, writes files
 * into the repository's own documentation directory, and is run when a page's pictures are out of
 * date. Pointing the suite at every `*.capture.ts` would put a lab requirement on the whole gate.
 *
 * `pnpm screenshots` starts the application and hands it this config; see `scripts/e2e-app.mjs`.
 */
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig } from "@playwright/test";

const OUTPUT_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "test-results");

export default defineConfig({
  testDir: ".",
  testMatch: /screenshots\.capture\.ts$/,
  outputDir: OUTPUT_DIR,
  workers: 1,
  fullyParallel: false,
  // Longer than the suite's: this run waits for a real connection, real command output and a real
  // sample of the host, where a test would be waiting for an assertion to hold.
  timeout: 180_000,
  expect: { timeout: 30_000 },
  reporter: [["list"]],
  use: {
    trace: "off",
    screenshot: "off",
  },
});
