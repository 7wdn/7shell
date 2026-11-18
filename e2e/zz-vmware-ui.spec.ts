/**
 * The VMware surfaces through the real window.
 *
 * The scan reads the Workstation inventory on this machine, so the case imports whatever it finds
 * instead of assuming a name. A machine without Workstation (or with an empty inventory) skips,
 * because the point here is the interface path — scan, group, workspace — not VMware's presence.
 */
import { chromium, expect, test, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

function sidebar(page: Page) {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

test("vmware import, group and workspace are drawn from the real inventory", async () => {
  test.setTimeout(240_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  // Standalone runs start before any vault exists.
  if (
    await page
      .getByRole("button", { name: "Create vault" })
      .isVisible()
      .catch(() => false)
  ) {
    await page.getByPlaceholder("At least 8 characters").fill("seven-shell-e2e");
    await page.getByPlaceholder("Type the password again").fill("seven-shell-e2e");
    await page.getByRole("button", { name: "Create vault" }).click();
  }
  await expect(page.getByRole("textbox", { name: "Search sessions" })).toBeVisible({ timeout: 30_000 });

  await page.locator("[data-vmware-scan]").click();
  const dialog = page.locator("[data-vmware-scan-dialog]");
  await expect(dialog).toBeVisible();

  const rows = dialog.locator("[data-vmware-scan-row]");
  try {
    await expect(rows.first()).toBeVisible({ timeout: 60_000 });
  } catch {
    test.skip(true, "this machine has no Workstation inventory to import");
  }

  const first = rows.first();
  const name = (await first.locator("td").nth(1).innerText()).split("\n")[0]?.trim() ?? "";
  expect(name.length, "the row names the machine").toBeGreaterThan(0);
  await first.locator('input[type="checkbox"]').first().check();
  await dialog.locator("[data-vmware-scan-import]").click();
  await expect(dialog).toHaveCount(0);

  // The default group is made for the import, and the machine lands inside it. The heading is the
  // group; the session row underneath may carry the same word as its guest OS label.
  await expect(sidebar(page).getByRole("heading", { name: "VMware", exact: true })).toBeVisible({
    timeout: 20_000,
  });
  const row = sidebar(page).locator('[data-row^="session:"]').filter({ hasText: name }).first();
  await expect(row).toBeVisible();

  await row.locator("button").first().click();

  // The workspace: the REST card and the host-location card are what this rewrite added.
  await expect(page.locator("[data-vmrest]")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator("[data-vmware-locations]")).toBeVisible();
  await expect(page.locator("[data-vmrest-save]")).toBeVisible();
  await expect(page.locator("[data-vmware-add-folder]")).toBeVisible();
});
