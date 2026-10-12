/**
 * The pictures the README and the documentation site show, taken from the real window.
 *
 * A capture rather than a set of hand-made images, and a capture rather than a mock: every one of
 * these is the application someone can install, connected to a real sshd in the lab, with the
 * window's own chrome, its own fonts and its own colours. A drawing of a screen ages into a lie the
 * moment the screen changes, and nothing would catch it — where a stale capture is at least the
 * application as it was.
 *
 * Run it with `pnpm screenshots`, which starts the application pointed at a throwaway data
 * directory and needs the SSH lab (`make lab-up`) on the port `SEVENSHELL_TEST_*` names.
 */
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
/** Where the site and the READMEs read them from: VitePress serves this directory as `/screenshots`. */
const OUTPUT = path.resolve(HERE, "..", "docs", "public", "screenshots");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

/** The vault this run creates. Thrown away with the data directory the launcher makes. */
const VAULT_PASSWORD = "seven-shell-capture";

/**
 * One window, in order: the gate, the first connection, the workspace.
 *
 * Serial because each shot depends on the one before it — the workspace has no terminal without the
 * session, and the session has no vault without the gate — and because a second worker would drive
 * the same window.
 */
test.describe.configure({ mode: "serial" });

async function openApp(): Promise<{ page: Page; close: () => Promise<void> }> {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }

  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  return { page, close: () => browser.close() };
}

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

/** Writes one shot at a size the README can print, whatever size the window happens to be. */
async function shot(page: Page, name: string): Promise<void> {
  await mkdir(OUTPUT, { recursive: true });
  await page.screenshot({ path: path.join(OUTPUT, `${name}.png`) });
}

test("the gate: the screen a first run opens on", async ({}, testInfo) => {
  const { page, close } = await openApp();
  try {
    // Nothing is filled in for this one on purpose: an empty form is what a person sees first, and
    // a picture of somebody's typed password is a picture this project would rather not publish.
    await expect(page.getByRole("button", { name: "Create vault" })).toBeVisible();
    await expect(page.getByPlaceholder("At least 8 characters")).toBeVisible();
    await shot(page, "create-vault");

    await page.getByPlaceholder("At least 8 characters").fill(VAULT_PASSWORD);
    await page.getByPlaceholder("Type the password again").fill(VAULT_PASSWORD);
    await page.getByRole("button", { name: "Create vault" }).click();
    await expect(sidebar(page).getByRole("button", { name: "New session" })).toBeVisible();
  } finally {
    await close();
  }
  testInfo.annotations.push({ type: "shot", description: "create-vault.png" });
});

test("the first connection: the host key is a question, not a warning to dismiss", async ({}, testInfo) => {
  const host = process.env.SEVENSHELL_TEST_HOST;
  const port = process.env.SEVENSHELL_TEST_PORT ?? "22";
  const user = process.env.SEVENSHELL_TEST_USER;
  const password = process.env.SEVENSHELL_TEST_PASSWORD;
  test.skip(!host || !user || !password, "the SSH lab is not configured: set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");

  const { page, close } = await openApp();
  try {
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill("lab");
    await editor.getByPlaceholder("example.com").fill(host as string);
    await editor.locator("#session-port").fill(port);
    await editor.getByPlaceholder("root").fill(user as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(password as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    await sidebar(page).getByTitle("Open a terminal for lab").click();

    // The prompt, with the fingerprint on it: this is the shot the security model is about, so it is
    // taken before anything is trusted.
    const trust = page.getByRole("button", { name: "Trust this key and connect" });
    await expect(trust).toBeVisible({ timeout: 60_000 });
    await shot(page, "host-key-prompt");

    await trust.click();
    testInfo.annotations.push({ type: "shot", description: "host-key-prompt.png" });
  } finally {
    await close();
  }
});

test("the workspace: a terminal over a real host, with the files beside it", async ({}, testInfo) => {
  const { page, close } = await openApp();
  try {
    await page.locator(".xterm").click({ timeout: 60_000 });
    // Real output from the real host: what a terminal looks like matters more in this picture than
    // which commands produced it, and these three are the ones whose output is worth reading.
    await page.keyboard.type("uname -sr && uptime && df -h | head -5\n");
    await page.waitForTimeout(4000);

    // And the file panel, pointed at a directory somebody would actually be looking at. A first
    // connection opens it at the far side's root, which on the container this runs against is a
    // scratch pad that the live tests have been writing into since the day it was created.
    const pathBar = page.getByPlaceholder("Path");
    if (await pathBar.isVisible().catch(() => false)) {
      await pathBar.fill("/etc");
      await pathBar.press("Enter");
      await expect(pathBar).toHaveValue("/etc");
      await page.waitForTimeout(3000);
    }

    await shot(page, "workspace");
    testInfo.annotations.push({ type: "shot", description: "workspace.png" });
  } finally {
    await close();
  }
});
