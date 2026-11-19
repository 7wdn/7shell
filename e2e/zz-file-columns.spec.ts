/**
 * A name column in a narrow listing that scrolls sideways.
 *
 * The listing scrolls as one line: the name column starts at the left, the columns after it follow,
 * and none of them is drawn over another. A frozen overlay was tried and produced text from the
 * columns passing underneath at the boundary, so this case checks the simpler contract — the name
 * is the leftmost column at the start, the next column begins after it, and scrolling moves the
 * whole line rather than leaving an overlay behind.
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

test("the name column does not cover the columns beside it", async () => {
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

  const gate = page.getByRole("heading", { name: "Create the vault" });
  const sessionList = page.locator("[data-session-list]");
  await expect(gate.or(sessionList)).toBeVisible({ timeout: 60_000 });
  if (await gate.isVisible()) {
    const password = "e2e-file-columns-password";
    await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
    await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
    await page.getByRole("button", { name: "Create vault" }).click();
    await expect(sessionList).toBeVisible({ timeout: 60_000 });
  }

  const name = `columns-${Date.now() % 100000}`;
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

  const crumbs = page.getByRole("navigation", { name: "Path" }).filter({ visible: true });
  await expect(crumbs).toBeVisible({ timeout: 30_000 });

  // A side dock is narrower than the columns ask for, which is what makes the listing scroll
  // sideways rather than squeezing the name away.
  await page.locator("[data-panel-placement]").filter({ visible: true }).click();
  await page.getByRole("menuitemcheckbox", { name: "Right", exact: true }).click();
  const panel = page.locator("[data-terminal-panel]").filter({ visible: true });
  await expect(panel).toHaveAttribute("data-side", "right", { timeout: 10_000 });

  // A side dock draws the listing without the directory tree: the docked widths leave no room for
  // both, and the reference docked panel is a single list for the same reason.
  await expect(page.locator("[data-files-tree]").filter({ visible: true })).toHaveCount(0);
  await expect(page.locator('nav[aria-label="Directories"]').filter({ visible: true })).toHaveCount(0);

  const listing = page.getByRole("listbox", { name: "Files", exact: true }).filter({ visible: true });
  await expect(listing.getByRole("option").first()).toBeVisible({ timeout: 30_000 });

  // The docked list opens with the reference's three columns, which fit the panel. Widening the
  // name past the panel is what makes the line scroll, and is the state this case is about.
  const nameEdge = listing
    .locator("[data-listing-header] [data-column-divider][data-column='name']")
    .filter({ visible: true });
  await expect(nameEdge).toBeVisible();
  const nameWidth = (): Promise<number> =>
    listing.evaluate((element) => {
      const name = element.querySelector<HTMLElement>('[data-column-cell="name"]');
      return name?.getBoundingClientRect().width ?? 0;
    });
  const widthBefore = await nameWidth();
  const grip = await nameEdge.boundingBox();
  if (!grip) {
    throw new Error("the name column's edge has no box");
  }
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 + 500, grip.y + grip.height / 2, { steps: 8 });
  await page.mouse.up();
  expect(await nameWidth(), "dragging the name edge did not widen the name column").toBeGreaterThan(widthBefore);

  const measured = await listing.evaluate((element) => {
    const header = element.querySelector<HTMLElement>("[data-listing-header]");
    const nameHeader = header?.querySelector<HTMLElement>('[data-column-cell="name"]');
    if (!header || !nameHeader) {
      throw new Error("the listing has no name header to measure");
    }

    element.scrollLeft = 0;
    const listBox = element.getBoundingClientRect();
    const startNameBox = nameHeader.getBoundingClientRect();

    element.scrollLeft = element.scrollWidth;
    const columns = [...header.querySelectorAll<HTMLElement>(":scope > span")]
      .map((cell) => cell.getBoundingClientRect())
      .filter((box) => box.right > listBox.left && box.left < listBox.right)
      .sort((left, right) => left.left - right.left);
    const overlaps = columns.some(
      (box, index) => index > 0 && box.left < (columns[index - 1]?.right ?? box.left) - 0.5,
    );

    return {
      overflowing: element.scrollWidth > element.clientWidth,
      scrolled: element.scrollLeft > 0,
      position: getComputedStyle(nameHeader).position,
      nameLeft: startNameBox.left,
      listLeft: listBox.left,
      nameWidth: startNameBox.width,
      overlaps,
    };
  });

  expect(measured.overflowing, "the panel is not narrow enough to make the listing scroll").toBe(true);
  expect(measured.scrolled, "the listing did not move sideways").toBe(true);
  expect(measured.position, "the name column must not be an overlay").not.toBe("sticky");
  expect(
    Math.abs(measured.nameLeft - (measured.listLeft + 6)),
    "the name column is not the leftmost column at the start",
  ).toBeLessThanOrEqual(2);
  expect(measured.nameWidth).toBeGreaterThan(0);
  expect(measured.overlaps, "two visible columns are drawn over each other").toBe(false);
});
