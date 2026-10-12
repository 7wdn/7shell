#!/usr/bin/env node
// Runs the end-to-end tests against the real application.
//
//   pnpm app:e2e
//   pnpm app:e2e --config e2e/playwright.capture.config.ts    (the screenshots, see package.json)
//
// Three things happen here, and each is deliberate:
//
// 1. The application is pointed at a throwaway data directory, so a test run cannot touch the vault,
//    the settings or the known hosts of whoever is running it.
// 2. The webview is asked to open its debugging port. WebView2 is Chromium, so that port is a
//    normal DevTools endpoint and the tests attach to it.
// 3. The interface is pinned to English first, so the selectors in the tests do not depend on the
//    language of the machine running them.

import { spawn, spawnSync } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import { DEV_PORT, listenersOn, portIsFree } from "./lib/dev-server.mjs";
import { REPO_ROOT } from "./lib/descriptors.mjs";

/** Which Playwright config to run under. Anything but the suite's own is asked for explicitly. */
const CONFIG = (() => {
  const index = process.argv.indexOf("--config");
  return index === -1 ? "e2e/playwright.config.ts" : process.argv[index + 1];
})();

const DATA_DIR = path.join(REPO_ROOT, "e2e", ".run", "config");
/**
 * Where the seeded download folder is.
 *
 * Under the run's own directory rather than the person's, and named for the run so a test can look
 * inside it: the folder is made by the application, and "it made one" is one of the things the
 * end-to-end run checks.
 */
const DOWNLOAD_DIR = path.join(REPO_ROOT, "e2e", ".run", "downloads", "fsdownload");
const DEBUG_PORT = Number(process.env.SEVENSHELL_E2E_PORT ?? 9222);
const CDP_URL = `http://127.0.0.1:${DEBUG_PORT}`;

/**
 * Clears whatever a previous run left behind out of the way.
 *
 * This is a developer tool being asked to test the application, so the application being open is
 * expected rather than an error: it gets closed, the run continues, and the log says what was
 * closed. The alternative — asking whoever is at the keyboard to close a window first — makes a
 * command that could have just worked depend on somebody else.
 *
 * What gets closed is the port, not just the application. An interrupted run leaves its dev server
 * behind, and that server is what makes this run's `tauri dev` die with "Port 1420 is already in
 * use" while the window it had already opened loads the interface from the stale one.
 */
async function closeRunningInstance() {
  console.log("e2e: an instance is running; closing it for the test run");

  if (process.platform === "win32") {
    spawnSync("taskkill", ["/F", "/IM", "sevenshell-desktop.exe"], { stdio: "ignore", shell: true });
  }

  for (const pid of listenersOn(DEV_PORT)) {
    spawnSync("taskkill", ["/F", "/T", "/PID", pid], { stdio: "ignore", shell: true });
  }

  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (await portIsFree(DEV_PORT)) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  console.error(`e2e: something still holds port ${DEV_PORT}, and it is not the application`);
  process.exit(2);
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

/**
 * Stops the application, its window, and the dev server it was started with.
 *
 * Synchronously, because every caller exits immediately afterwards. The tree rather than the process
 * alone, because what is started here is a shell over `tauri dev` over the dev server: stopping at
 * the top leaves the dev server holding the port, and the run after this one then loads the
 * interface from a server its own command never started.
 */
function stopApp(app) {
  if (process.platform !== "win32") {
    app.kill();
    return;
  }

  if (app.pid) {
    spawnSync("taskkill", ["/F", "/T", "/PID", String(app.pid)], { stdio: "ignore", shell: true });
  }
  spawnSync("taskkill", ["/F", "/IM", "sevenshell-desktop.exe"], { stdio: "ignore", shell: true });
}

async function main() {
  if (!(await portIsFree(DEV_PORT))) {
    await closeRunningInstance();
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

  // Where downloads land, written before the application starts.
  //
  // The one setting a test cannot set for itself: choosing a folder opens a *native* dialog, and a
  // driver cannot answer one of those. Seeding the file is also what makes the download path
  // testable at all — with a folder in force there is no dialog between the click and the bytes,
  // which is exactly the case the end-to-end run exists to check. The folder itself is deliberately
  // not created here: the application makes it, and the test is that it does.
  await writeFile(
    path.join(DATA_DIR, "downloads.json"),
    `${JSON.stringify(
      {
        schemaVersion: 1,
        current: DOWNLOAD_DIR,
        history: [DOWNLOAD_DIR, path.join(DATA_DIR, "older-downloads")],
      },
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

    // A name to run one case by, for the loop of writing a case and watching it: setting it is not
    // the default, and the whole suite is what CI and the gate run. Quoted because this runs through
    // a shell, where an unquoted name with a space in it would arrive as two arguments.
    const grep = process.env.SEVENSHELL_E2E_GREP;
    tests = await new Promise((resolve) => {
      const run = spawn(
        "pnpm",
        ["exec", "playwright", "test", "--config", CONFIG, ...(grep ? ["-g", `"${grep}"`] : [])],
        {
          cwd: REPO_ROOT,
          shell: true,
          stdio: "inherit",
          env: { ...process.env, SEVENSHELL_E2E_CDP: CDP_URL },
        },
      );
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
