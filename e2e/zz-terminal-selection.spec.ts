/**
 * Selection gestures in the real terminal renderer.
 *
 * A lost compatibility mouseup must not leave the renderer extending a word selection after the
 * pointer was released. The local shell keeps this independent of the SSH lab; only the second
 * mouseup is suppressed, so the browser still delivers the pointerup that should end the gesture.
 */
import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

/** The painted rectangles, rather than an internal selection model or a mocked terminal. */
async function selectionBoxes(surface: Locator): Promise<{ x: number; y: number; width: number; height: number }[]> {
  return surface.evaluate(async (element) => {
    // Selection changes are painted on the renderer's next animation frame.
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const origin = element.getBoundingClientRect();
    const rounded = (value: number): number => Math.round(value * 10) / 10;
    return [...element.querySelectorAll(".xterm-selection div")]
      .map((part) => part.getBoundingClientRect())
      .filter((part) => part.width > 0 && part.height > 0)
      .map((part) => ({
        x: rounded(part.left - origin.left),
        y: rounded(part.top - origin.top),
        width: rounded(part.width),
        height: rounded(part.height),
      }));
  });
}

test("terminal selections stop at release and recover when a mouseup is lost", async () => {
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
      const password = "e2e-terminal-selection-password";
      await page.getByRole("textbox", { name: "Password", exact: true }).fill(password);
      await page.getByRole("textbox", { name: "Confirm password" }).fill(password);
      await page.getByRole("button", { name: "Create vault" }).click();
      await expect(sessionList).toBeVisible({ timeout: 60_000 });
    }

    const name = `selection-${Date.now() % 100000}`;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.locator("#session-protocol").click();
    await page.getByRole("listbox").getByRole("option", { name: "Local terminal" }).click();
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);
    await sidebar(page).getByTitle(`Open a terminal for ${name}`).click();

    const input = page.getByRole("textbox", { name: "Terminal input" }).filter({ visible: true });
    await expect(input).toBeVisible({ timeout: 30_000 });
    await input.click();
    const marker = `${name} bravo charlie delta foxtrot golf hotel`;
    await page.keyboard.type(`echo "${marker}"`);
    await page.keyboard.press("Enter");
    const surface = page.locator(".terminal-surface").filter({ visible: true }).first();
    const printed = surface.locator(".xterm-rows > div", { hasText: marker }).filter({ hasNotText: 'echo "' }).last();
    await expect(printed).toBeVisible({ timeout: 20_000 });
    const line = await printed.boundingBox();
    if (!line) {
      throw new Error("the local shell did not draw the selection fixture");
    }
    const start = { x: line.x + 24, y: line.y + line.height / 2 };
    const nearX = line.x + 110;
    const farX = line.x + Math.min(330, line.width - 10);
    const tools = page.getByRole("toolbar", { name: "Selection actions" });

    await page.mouse.dblclick(start.x, start.y);
    await expect(tools).toBeVisible();
    const normal = await selectionBoxes(surface);
    expect(normal.length, "double-clicking a word painted a selection").toBeGreaterThan(0);
    await page.mouse.move(farX, start.y);
    expect(await selectionBoxes(surface), "a released double-click must not follow ordinary pointer movement").toEqual(
      normal,
    );
    await page.mouse.move(farX, start.y + line.height * 2);
    expect(await selectionBoxes(surface), "the released selection also stays put across rows").toEqual(normal);

    await page.evaluate(() => {
      let releases = 0;
      const dropSecondRelease = (event: MouseEvent): void => {
        // A recovery mouseup is synthetic; only the physical gesture is interrupted here.
        if (!event.isTrusted || event.button !== 0) {
          return;
        }
        releases += 1;
        if (releases === 2) {
          event.stopImmediatePropagation();
          document.documentElement.dataset.selectionTestDroppedRelease = "true";
          window.removeEventListener("mouseup", dropSecondRelease, true);
        }
      };
      window.addEventListener("mouseup", dropSecondRelease, true);
    });
    await page.mouse.dblclick(start.x, start.y);
    await expect(page.locator("html")).toHaveAttribute("data-selection-test-dropped-release", "true");
    await expect(tools).toBeVisible();
    const recovered = await selectionBoxes(surface);
    expect(recovered.length, "recovering a release preserves the word that was selected").toBeGreaterThan(0);
    await page.mouse.move(farX, start.y + line.height * 2);
    expect(await selectionBoxes(surface), "a lost mouseup must not leave a live word-selection drag").toEqual(
      recovered,
    );

    // Deliver the second press without either release. The driver still has its button up, so the
    // next real move tells the terminal it can end the drag even with no release event to rely on.
    await surface.locator(".xterm").dispatchEvent("mousedown", {
      button: 0,
      buttons: 1,
      clientX: start.x,
      clientY: start.y,
      detail: 2,
    });
    const withoutRelease = await selectionBoxes(surface);
    expect(
      withoutRelease.length,
      "the double-click selected a word before both release events were lost",
    ).toBeGreaterThan(0);
    await page.mouse.move(farX - 20, start.y + line.height * 2);
    expect(
      await selectionBoxes(surface),
      "movement with no buttons ends a drag even when both releases were lost",
    ).toEqual(withoutRelease);

    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(nearX, start.y, { steps: 3 });
    const near = await selectionBoxes(surface);
    expect(near.length, "holding the button still starts a selection").toBeGreaterThan(0);
    await page.mouse.move(farX, start.y, { steps: 3 });
    const far = await selectionBoxes(surface);
    const area = (boxes: typeof far): number => boxes.reduce((total, box) => total + box.width * box.height, 0);
    expect(area(far), "the held drag keeps extending through more terminal cells").toBeGreaterThan(area(near));
    await page.mouse.up();
    await page.mouse.move(start.x, start.y + line.height * 2);
    expect(await selectionBoxes(surface), "releasing a regular drag freezes its selection").toEqual(far);

    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(nearX, start.y, { steps: 3 });
    const beforeBlur = await selectionBoxes(surface);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await page.mouse.move(farX, start.y, { steps: 3 });
    expect(await selectionBoxes(surface), "losing the window ends the drag without clearing its selection").toEqual(
      beforeBlur,
    );
    await page.mouse.up();
  } finally {
    // A failed assertion can leave the driver's button held or a capture listener installed.
    await page.mouse.up().catch(() => {});
    try {
      const tabs = page.locator("[data-workspace-tabs] [data-tab]");
      for (let guard = 0; guard < 10 && (await tabs.count()) > 0; guard += 1) {
        const remaining = await tabs.count();
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
