#!/usr/bin/env node
// Runs the end-to-end tests against the real application.
//
//   pnpm app:e2e
//
// Three things happen here, and each is deliberate:
//
// 1. The application is pointed at a throwaway data directory, so a test run cannot touch the vault,
//    the settings or the known hosts of whoever is running it.
// 2. The webview is asked to open its debugging port. WebView2 is Chromium, so that port is a
//    normal DevTools endpoint and the tests attach to it.
// 3. The interface is pinned to English first, so the selectors in the tests do not depend on the
//    language of the machine running them.

import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import { REPO_ROOT } from "./lib/descriptors.mjs";

const DATA_DIR = path.join(REPO_ROOT, "e2e", ".run", "config");
const DEBUG_PORT = Number(process.env.SEVENSHELL_E2E_PORT ?? 9222);
const CDP_URL = `http://127.0.0.1:${DEBUG_PORT}`;

/**
 * Refuses to start when the development port is taken.
 *
 * The application the tests drive is a separate instance with its own data directory, and it needs
 * that port. If somebody already has the application open — which is normal, it is a desktop
 * application — starting a second one fails deep inside the webview with an error that says nothing
 * about the cause. Saying it here, in one line, is the difference between a puzzle and an
 * instruction.
 */
async function portIsFree(port) {
  return new Promise((resolve) => {
    const probe = createServer();
    probe.once("error", () => resolve(false));
    probe.once("listening", () => probe.close(() => resolve(true)));
    probe.listen(port, "127.0.0.1");
  });
}

/** Waits for the debugging endpoint to answer, or gives up saying what it waited for. */
async function waitForCdp(timeoutMs) {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${CDP_URL}/json/version`);
      if (response.ok) {
        return await response.json();
      }
    } catch {
      // Not up yet. The application takes as long as it takes to build and start.
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }

  throw new Error(`${CDP_URL}/json/version never answered within ${timeoutMs / 1000}s`);
}

function stopApp(app) {
  app.kill();
  if (process.platform === "win32") {
    spawn("taskkill", ["/F", "/IM", "sevenshell-desktop.exe"], { stdio: "ignore", shell: true });
  }
}

async function main() {
  if (!(await portIsFree(1420))) {
    console.error(
      "e2e: the development port 1420 is taken — most likely by the application itself. Close it and run this again: the tests need their own instance, with its own data directory.",
    );
    process.exit(2);
  }

  // Every run starts from nothing: a previous run's vault would turn "create the vault" into
  // "unlock the vault", and the tests would be describing a different application each time.
  await rm(path.dirname(DATA_DIR), { recursive: true, force: true });
  await mkdir(DATA_DIR, { recursive: true });
  console.log(`e2e: data directory ${DATA_DIR}`);
  await writeFile(
    path.join(DATA_DIR, "settings.json"),
    `${JSON.stringify(
      { schemaVersion: 1, themeMode: "dark", terminalTheme: "system", filesLayout: "tree", locale: "en" },
      null,
      2,
    )}\n`,
    "utf8",
  );

  const app = spawn("pnpm", ["dev:tauri"], {
    cwd: REPO_ROOT,
    shell: true,
    stdio: ["ignore", "inherit", "inherit"],
    env: {
      ...process.env,
      SEVENSHELL_CONFIG_DIR: DATA_DIR,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${DEBUG_PORT}`,
    },
  });

  // An interrupted run must not leave a window behind: it is indistinguishable from the real
  // application to whoever is looking at the screen.
  process.on("SIGINT", () => {
    stopApp(app);
    process.exit(130);
  });
  process.on("SIGTERM", () => {
    stopApp(app);
    process.exit(143);
  });

  console.log("e2e: starting the application (a window will appear, pointed at the test data)");
  let tests = 1;
  try {
    const version = await waitForCdp(180_000);
    console.log(`attached to ${version.Browser ?? "the webview"} at ${CDP_URL}`);

    tests = await new Promise((resolve) => {
      const run = spawn("pnpm", ["exec", "playwright", "test", "--config", "e2e/playwright.config.ts"], {
        cwd: REPO_ROOT,
        shell: true,
        stdio: "inherit",
        env: { ...process.env, SEVENSHELL_E2E_CDP: CDP_URL },
      });
      run.on("exit", (code) => resolve(code ?? 1));
    });
  } finally {
    // The application is a child process group: killing the shell leaves the window behind.
    stopApp(app);
    console.log("e2e: the application was stopped");
  }

  process.exit(tests);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
