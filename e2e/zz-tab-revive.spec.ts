/**
 * Disconnecting a tab, and reconnecting it into the same screen.
 *
 * Environment-gated on the SSH lab, because the claim is about a real shell: the output of the
 * first session has to still be on screen after the second one connects. The tab's identity is
 * the other half — one tab before the disconnect, one tab after the reconnect, and the same one.
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

function tabs(page: Page): Locator {
  return page.locator("[data-workspace-tabs] [data-tab]");
}

test("a disconnected tab keeps its screen, and reconnecting writes into the same one", async () => {
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
    // Alone, this case is the first thing the window sees: the vault gate is what it starts on.
    const gate = page.getByRole("heading", { name: "Create the vault" });
    const sessionList = page.locator("[data-session-list]");
    await expect(gate.or(sessionList)).toBeVisible({ timeout: 60_000 });
    if (await gate.isVisible()) {
      const password = "e2e-tab-revive-password";
      await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
      await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
      await page.getByRole("button", { name: "Create vault" }).click();
      await expect(sessionList).toBeVisible({ timeout: 60_000 });
    }

    const stamp = `revive-${Date.now() % 100000}`;
    const name = `revive-${stamp}`;
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
    await expect(question.or(page.getByRole("textbox", { name: "Terminal input" })).first()).toBeVisible({
      timeout: 30_000,
    });
    if (await question.isVisible()) {
      await question.click();
    }

    // Something of this run's own on screen, which is what has to survive the reconnect.
    const marker = `${stamp}-kept`;
    const screen = page.locator(".xterm-rows").filter({ visible: true }).first();
    const input = page.getByRole("textbox", { name: "Terminal input" });
    await expect(input).toBeVisible({ timeout: 30_000 });
    await input.click();
    await page.keyboard.type(`echo ${marker}`);
    await page.keyboard.press("Enter");
    await expect(screen).toContainText(marker, { timeout: 20_000 });

    // One tab, and it is the one this case is about.
    await expect(tabs(page)).toHaveCount(1);

    await tabs(page).first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Disconnect", exact: true }).click();
    await expect(page.getByText("Disconnected.", { exact: true })).toBeVisible({ timeout: 20_000 });
    await expect(tabs(page), "the tab stays where it was").toHaveCount(1);
    await expect(screen, "and keeps the last screen").toContainText(marker);

    await page.getByRole("button", { name: "Reconnect", exact: true }).click();

    // The same tab, and the same scrollback: the new session writes below what was already there.
    await expect(tabs(page), "a reconnect revives the tab rather than adding one").toHaveCount(1);
    await expect(screen, "the first session's output is still on screen").toContainText(marker, {
      timeout: 30_000,
    });
    await expect
      .poll(async () => (await page.getByRole("textbox", { name: "Terminal input" }).count()) > 0, {
        message: "and the new shell is there to type into",
      })
      .toBe(true);
  } finally {
    try {
      const drawn = tabs(page);
      for (let guard = 0; guard < 10 && (await drawn.count()) > 0; guard += 1) {
        const remaining = await drawn.count();
        await drawn
          .last()
          .locator("button")
          .last()
          .click({ timeout: 5_000 })
          .catch(() => {});
        await expect(drawn)
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
