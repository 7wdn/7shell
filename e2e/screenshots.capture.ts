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
import { execFileSync } from "node:child_process";
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
 * One window, in order: the gate, the first connection, the workspace, the container workspace and
 * the terminal scheme catalogue.
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

async function hideMonitor(page: Page): Promise<void> {
  const close = page.getByRole("button", { name: "Hide the monitor" });
  if (await close.isVisible().catch(() => false)) {
    await close.click();
  }
}

/** Saves a session without opening it, so the sidebar shows the kinds a real list contains. */
async function saveSession(page: Page, name: string, kind: string): Promise<void> {
  await sidebar(page).getByRole("button", { name: "New session" }).click();
  const editor = page.getByRole("dialog");
  await editor.getByPlaceholder("Production web server").fill(name);
  await editor.locator("#session-protocol").click();
  await page.getByRole("listbox").getByRole("option", { name: kind, exact: true }).click();
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor).toHaveCount(0);
}

/** Writes one shot at a size the README can print, whatever size the window happens to be. */
async function shot(page: Page, name: string): Promise<void> {
  await mkdir(OUTPUT, { recursive: true });
  await page.screenshot({ path: path.join(OUTPUT, `${name}.png`) });
}

// No fixture arguments: each test opens the application itself. Playwright requires the first
// argument, when there is one, to destructure fixtures; `test.info()` names the shot instead.
test("the gate: the screen a first run opens on", async () => {
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
  test.info().annotations.push({ type: "shot", description: "create-vault.png" });
});

test("the first connection: the host key is a question, not a warning to dismiss", async () => {
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
    test.info().annotations.push({ type: "shot", description: "host-key-prompt.png" });
  } finally {
    await close();
  }
});

test("the workspace: a terminal over a real host, with the files beside it", async () => {
  const { page, close } = await openApp();
  try {
    // A session list with more than one kind in it, and a second tab: the picture is of the
    // workspace somebody works in, not of the first terminal the capture happened to open.
    await saveSession(page, "Local PowerShell", "Local terminal");
    await saveSession(page, "docker-local", "Docker");
    await saveSession(page, "k3s lab", "Kubernetes");

    await sidebar(page).getByTitle("Open a terminal for Local PowerShell").click();
    const tabs = page.locator("[data-workspace-tabs]");
    await expect(tabs.getByRole("button", { name: "Local PowerShell", exact: true })).toBeVisible({
      timeout: 30_000,
    });
    await tabs.getByRole("button", { name: "lab", exact: true }).click();

    await page.locator(".xterm").filter({ visible: true }).first().click({ timeout: 60_000 });
    // Real output from the real host. `ls /root` is there for the `Permission denied` it prints,
    // which is what a keyword rule looks like without a made-up demonstration.
    await page.keyboard.type("uname -sr && uptime\n");
    await page.keyboard.type("df -h | head -5\n");
    await page.keyboard.type("ls --color=auto /etc | head -12\n");
    await page.keyboard.type("ls /root\n");
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
    test.info().annotations.push({ type: "shot", description: "workspace.png" });
  } finally {
    await close();
  }
});

test("the container workspace: a real engine and its containers", async () => {
  let docker = true;
  try {
    execFileSync("docker", ["ps"], { stdio: "ignore" });
  } catch {
    docker = false;
  }
  test.skip(!docker, "Docker is not running on this machine");

  const { page, close } = await openApp();
  try {
    await hideMonitor(page);
    await sidebar(page).getByTitle("Open a terminal for docker-local").click();

    const rail = page.getByRole("navigation", { name: "Container workspace" });
    await expect(rail).toBeVisible({ timeout: 60_000 });
    await rail.getByRole("button", { name: /^Containers/ }).click();
    // The engine on this machine also runs whatever else its owner started; the picture is of the
    // lab this repository builds, not of the person's desktop.
    await page.getByPlaceholder("Filter by name, image or ID").fill("sevenshell-lab");
    await expect(page.getByText("sevenshell-lab-sshd").first()).toBeVisible({ timeout: 60_000 });
    await page.waitForTimeout(1500);

    await shot(page, "containers");
    test.info().annotations.push({ type: "shot", description: "containers.png" });
  } finally {
    await close();
  }
});

test("the terminal schemes: the catalogue a terminal is dressed from", async () => {
  const { page, close } = await openApp();
  try {
    await hideMonitor(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await page
      .getByRole("navigation", { name: "Settings categories" })
      .getByRole("button", { name: "SSH terminal" })
      .click();

    await page.getByRole("radio", { name: /^Colour schemes/ }).click();
    await expect(page.locator("[data-terminal-schemes]")).toBeVisible();
    await page.getByRole("button", { name: "Scheme for dark terminals" }).click();
    await expect(page.getByRole("listbox")).toBeVisible();
    // The settings page was scrolled sideways to bring the slot into view. Put the page behind the
    // dialog back where a person would see it, so the shot is of the catalogue rather than of a
    // scroll position.
    await page.locator("[data-terminal-schemes]").evaluate((element) => {
      for (let node = element.parentElement; node; node = node.parentElement) {
        if (node.scrollWidth > node.clientWidth) {
          node.scrollLeft = 0;
        }
      }
    });
    await page.waitForTimeout(500);

    await shot(page, "terminal-schemes");
    test.info().annotations.push({ type: "shot", description: "terminal-schemes.png" });
  } finally {
    await close();
  }
});
