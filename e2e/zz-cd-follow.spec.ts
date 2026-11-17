/**
 * The file panel under a terminal, following the shell it belongs to.
 *
 * Environment-gated on the SSH lab: the claim is about a real shell's `cd`, and the directory it
 * moves to has to be a real one. The fixture is `/data/test`, made here through the lab container
 * because no compose recipe ships it; the case creates its own session and drives both ways a line
 * reaches a shell — the keyboard, and the command bar that writes straight into the pty.
 */
import { execFileSync } from "node:child_process";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

const LAB_HOST = process.env.SEVENSHELL_TEST_HOST;
const LAB_PORT = process.env.SEVENSHELL_TEST_PORT ?? "22";
const LAB_USER = process.env.SEVENSHELL_TEST_USER;
const LAB_PASSWORD = process.env.SEVENSHELL_TEST_PASSWORD;

/** The lab container, which is where the directory this case `cd`s into lives. */
const LAB_CONTAINER = "sevenshell-lab-sshd";

/** Makes the fixture, and says whether it could be made. Read as root: `/data` belongs to root. */
function makeFixture(): boolean {
  try {
    execFileSync(
      "docker",
      ["exec", "-u", "root", LAB_CONTAINER, "sh", "-lc", "mkdir -p /data/test && touch /data/test/a.txt && chmod -R 755 /data"],
      { stdio: "ignore" },
    );
    return true;
  } catch {
    return false;
  }
}

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

function sessionRow(page: Page, name: string): Locator {
  return sidebar(page).getByTitle(`Open a terminal for ${name}`);
}

test("the panel follows a cd into a subdirectory, not only into a top-level one", async () => {
  test.skip(
    !LAB_HOST || !LAB_USER || !LAB_PASSWORD,
    "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD",
  );
  test.skip(!makeFixture(), "the SSH lab container is not reachable; the /data/test fixture needs it");

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    // Run alone, this case is the first thing the window sees; the suite's own vault case is what
    // normally creates it, and waiting for that one here would make this case unrunnable on its own.
    // The window takes a moment to say which of the two it is showing, so both are waited for rather
    // than checked once.
    const gate = page.getByRole("heading", { name: "Create the vault" });
    const sessionList = page.locator("[data-session-list]");
    await expect(gate.or(sessionList)).toBeVisible({ timeout: 60_000 });
    if (await gate.isVisible()) {
      const password = "e2e-cd-follow-password";
      await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
      await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
      await page.getByRole("button", { name: "Create vault" }).click();
      await expect(sessionList).toBeVisible({ timeout: 60_000 });
    }

    const name = `cd-follow-${Date.now() % 100000}`;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(LAB_HOST as string);
    await editor.locator("#session-port").fill(LAB_PORT);
    await editor.getByPlaceholder("root").fill(LAB_USER as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    const question = page.getByRole("button", { name: "Trust this key and connect" });
    await sessionRow(page, name).click();
    await expect(question).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }

    // The split layout opens the file half under the terminal; the path bar is its address.
    const fileToggle = page.getByRole("main").getByRole("button", { name: "Files", exact: true });
    await expect(fileToggle).toBeVisible({ timeout: 30_000 });
    // The split layout opens the files half on its own. It takes a round trip to open, so it is
    // waited for before the toggle is touched: clicking a panel that is already opening closes it,
    // and the path bar then never appears at all.
    const pathBar = page.getByPlaceholder("Path").filter({ visible: true });
    await expect(pathBar)
      .toBeVisible({ timeout: 20_000 })
      .catch(async () => {
        await fileToggle.click();
        await expect(pathBar).toBeVisible({ timeout: 30_000 });
      });
    await expect(pathBar).not.toHaveValue("", { timeout: 30_000 });

    const screen = page.locator(".xterm-screen").filter({ visible: true }).first();
    await screen.click();

    await page.keyboard.type("cd /data");
    await page.keyboard.press("Enter");
    await expect(pathBar).toHaveValue("/data", { timeout: 20_000 });

    await page.keyboard.type("cd /data/test");
    await page.keyboard.press("Enter");
    await expect(pathBar).toHaveValue("/data/test", { timeout: 20_000 });
    await expect(page.getByText("a.txt").first()).toBeVisible({ timeout: 20_000 });

    // The bar above the panel writes straight into the pty, so this `cd` never passes through the
    // keyboard and the pane's own line reader cannot see it. The panel still has to follow.
    const bar = page.getByRole("combobox", { name: "Command input" }).filter({ visible: true });
    await bar.fill("cd /etc");
    await page.getByRole("button", { name: "Send to this session" }).click();
    await expect(pathBar).toHaveValue("/etc", { timeout: 20_000 });
  } finally {
    // One window outlives every case: the terminal opened here is closed, and the window is
    // reloaded, so the case after this one does not start on this one's connection.
    try {
      const tabs = page.locator("[data-workspace-tabs] [data-tab]");
      for (let guard = 0; guard < 10; guard += 1) {
        const remaining = await tabs.count();
        if (remaining === 0) {
          break;
        }
        await tabs
          .last()
          .locator("button")
          .last()
          .click({ timeout: 5_000 })
          .catch(() => {});
        await expect(tabs)
          .toHaveCount(remaining - 1, { timeout: 5_000 })
          .catch(() => {});
      }
      await page.reload({ waitUntil: "domcontentloaded", timeout: 30_000 });
      await expect(page.locator("[data-session-list]")).toBeVisible({ timeout: 60_000 });
    } catch (error) {
      console.warn(`e2e: the window was not put back after the case: ${String(error)}`);
    }
    await browser.close();
  }
});
