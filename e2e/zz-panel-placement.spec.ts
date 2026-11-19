/**
 * Where the file panel sits, as a person moves it.
 *
 * Environment-gated on the SSH lab: the panel is an SFTP browser, so a real server is what makes
 * the four positions real. The case checks the two docked sides, the existing standalone tab, and
 * that a side chosen while the files are showing moves the same browser rather than opening a
 * second one.
 */
import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

const LAB_HOST = process.env.SEVENSHELL_TEST_HOST;
const LAB_PORT = process.env.SEVENSHELL_TEST_PORT ?? "22";
const LAB_USER = process.env.SEVENSHELL_TEST_USER;
const LAB_PASSWORD = process.env.SEVENSHELL_TEST_PASSWORD;

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

function sessionRow(page: Page, name: string): Locator {
  return sidebar(page).getByTitle(`Open a terminal for ${name}`);
}

/** Picks one position from the small menu beside Files. */
async function choosePlacement(page: Page, label: string): Promise<void> {
  await page.locator("[data-panel-placement]").filter({ visible: true }).click();
  await page.getByRole("menuitemcheckbox", { name: label, exact: true }).click();
}

test("the file panel moves between the two sides, the bottom and its own tab", async () => {
  test.skip(
    !LAB_HOST || !LAB_USER || !LAB_PASSWORD,
    "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD",
  );

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    const gate = page.getByRole("heading", { name: "Create the vault" });
    const sessionList = page.locator("[data-session-list]");
    await expect(gate.or(sessionList)).toBeVisible({ timeout: 60_000 });
    if (await gate.isVisible()) {
      const password = "e2e-panel-placement-password";
      await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
      await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
      await page.getByRole("button", { name: "Create vault" }).click();
      await expect(sessionList).toBeVisible({ timeout: 60_000 });
    }

    const name = `placement-${Date.now() % 100000}`;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(LAB_HOST as string);
    await editor.locator("#session-port").fill(LAB_PORT);
    await editor.getByPlaceholder("root").fill(LAB_USER as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    await sessionRow(page, name).click();
    const question = page.getByRole("button", { name: "Trust this key and connect" });
    await expect(question).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }

    // The split layout opens the browser under the terminal; its breadcrumbs are the proof that a
    // real directory listing is on screen before a position is chosen.
    const crumbs = page.getByRole("navigation", { name: "Path" }).filter({ visible: true });
    await expect(crumbs).toBeVisible({ timeout: 30_000 });
    const panel = page.locator("[data-terminal-panel]").filter({ visible: true });
    await expect(panel).toHaveAttribute("data-side", "bottom", { timeout: 10_000 });

    await choosePlacement(page, "Right");
    await expect(panel).toHaveAttribute("data-side", "right", { timeout: 10_000 });
    await expect(
      page.locator('[data-split-divider][aria-orientation="vertical"]').filter({ visible: true }),
    ).toHaveCount(1);

    await choosePlacement(page, "Left");
    await expect(panel).toHaveAttribute("data-side", "left", { timeout: 10_000 });
    await expect(crumbs).toBeVisible();

    await choosePlacement(page, "New file manager tab");
    await expect(page.locator("[data-terminal-panel]").filter({ visible: true })).toHaveCount(0, {
      timeout: 20_000,
    });
    await expect(page.locator("[data-workspace-tabs] [data-tab]")).toHaveCount(2, { timeout: 20_000 });
    await expect(crumbs).toBeVisible({ timeout: 20_000 });

    // Back to a docked side: the tab stays where it is, and a panel appears beside the terminal.
    // The terminal has to be the tab on screen, because the placement menu lives in its strip.
    await page.locator("[data-workspace-tabs] [data-tab]").first().locator("button").first().click();
    await choosePlacement(page, "Bottom");
    await expect(page.locator("[data-terminal-panel]").filter({ visible: true })).toHaveAttribute(
      "data-side",
      "bottom",
      { timeout: 20_000 },
    );
    // Moving back to a dock is a move: the tab the position opened goes with it.
    await expect(page.locator("[data-workspace-tabs] [data-tab]")).toHaveCount(1, { timeout: 20_000 });
  } finally {
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
