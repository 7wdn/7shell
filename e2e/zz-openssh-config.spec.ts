/**
 * A session whose host is an OpenSSH alias.
 *
 * The launcher seeds the run's own `~/.ssh/config` with one `Host e2e-lab` pointing at the SSH lab,
 * so the picker is reading a file whose contents this run chose. The session stores the alias; the
 * address, account and port come from the configuration when the connection is made.
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

function tabs(page: Page): Locator {
  return page.locator("[data-workspace-tabs] [data-tab]");
}

test("a host picked from ~/.ssh/config connects through its alias", async () => {
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
      const password = "e2e-openssh-password";
      await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
      await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
      await page.getByRole("button", { name: "Create vault" }).click();
      await expect(sessionList).toBeVisible({ timeout: 60_000 });
    }

    const stamp = `openssh-${Date.now() % 100000}`;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");

    await editor.locator("[data-ssh-config-picker]").click();
    const host = page.locator("[data-ssh-config-host]").filter({ hasText: "e2e-lab" });
    await expect(host).toBeVisible({ timeout: 60_000 });

    // The launcher seeds a second alias so the filter has a row to leave out.
    const filter = page.locator("[data-ssh-config-filter]");
    await filter.fill("shadow");
    await expect(page.locator("[data-ssh-config-host]")).toHaveCount(1);
    await expect(page.locator("[data-ssh-config-host]").filter({ hasText: "e2e-shadow" })).toBeVisible();
    await filter.fill("no-such-host");
    await expect(page.getByText("No hosts match this filter.")).toBeVisible();
    await filter.fill("");
    await expect(host).toBeVisible();

    await host.click();

    // The alias is what the session stores; the resolved values are shown beside it.
    await expect(editor.locator("#session-host")).toHaveValue("e2e-lab");
    await expect(editor.locator("#session-user")).toHaveValue(LAB_USER as string);
    await expect(editor.locator("#session-port")).toHaveValue(LAB_PORT);
    await expect(editor.getByText("The alias is saved instead of a copy of the address.")).toBeVisible();

    // The picker starts at key authentication, which the lab user has not been given. The alias
    // resolution is what this case is about, so the password is the credential.
    await editor.getByRole("combobox", { name: "Authentication" }).click();
    await page.getByRole("listbox").getByRole("option", { name: "Password" }).click();
    await editor.getByPlaceholder("Production web server").fill(stamp);
    await editor.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    const question = page.getByRole("button", { name: "Trust this key and connect" });
    await sidebar(page).getByTitle(`Open a terminal for ${stamp}`).click();
    await expect(question.or(page.getByRole("textbox", { name: "Terminal input" })).first()).toBeVisible({
      timeout: 30_000,
    });
    if (await question.isVisible()) {
      await question.click();
    }

    const input = page.getByRole("textbox", { name: "Terminal input" });
    await expect(input).toBeVisible({ timeout: 30_000 });
    await input.click();
    const marker = `${stamp}-ok`;
    await page.keyboard.type(`echo ${marker}`);
    await page.keyboard.press("Enter");
    await expect(page.locator(".xterm-rows").filter({ visible: true }).first()).toContainText(marker, {
      timeout: 20_000,
    });
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
