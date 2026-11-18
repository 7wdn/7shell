/**
 * The vault's automatic-unlock switch, through the real commands and the real system store.
 *
 * The whole-process half — a restart that opens the vault without the gate — is covered by
 * `src-tauri`'s `vault_keychain` integration test, which uses a real vault and the real DPAPI path.
 * This case is the other half: the place a person finds the switch, and the status it reports after
 * the store has actually accepted or removed the key.
 */
import { chromium, expect, test, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

async function readyShell(page: Page): Promise<void> {
  const create = page.getByRole("heading", { name: "Create the vault" });
  const unlock = page.getByRole("heading", { name: "Unlock the vault" });
  const shell = page.locator("[data-session-list]");
  await expect(create.or(unlock).or(shell)).toBeVisible({ timeout: 60_000 });

  if (await create.isVisible()) {
    const password = "e2e-keychain-password";
    await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
    await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
    await page.getByRole("button", { name: "Create vault" }).click();
  } else if (await unlock.isVisible()) {
    // The suite's first case creates the vault with this password; a standalone run creates its own.
    await page.getByRole("textbox", { name: "Password", exact: true }).fill("seven-shell-e2e");
    await page.getByRole("button", { name: "Unlock", exact: true }).click();
  }
  await expect(shell).toBeVisible({ timeout: 60_000 });
}

test("the system keychain switch turns automatic unlock on and off", async () => {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    await readyShell(page);

    await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Security", exact: true }).click();

    const toggle = page.locator("[data-vault-keychain]");
    await expect(toggle).toBeVisible({ timeout: 15_000 });

    if ((await toggle.getAttribute("aria-checked")) === "true") {
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-checked", "false", { timeout: 15_000 });
    }

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "true", { timeout: 15_000 });
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "false", { timeout: 15_000 });
  } finally {
    try {
      await page.reload({ waitUntil: "domcontentloaded", timeout: 30_000 });
      await expect(page.locator("[data-session-list]")).toBeVisible({ timeout: 60_000 });
    } catch (error) {
      console.warn(`e2e: the window was not put back after the case: ${String(error)}`);
    }
    await browser.close();
  }
});
