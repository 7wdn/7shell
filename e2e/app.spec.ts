/**
 * What the running application does, driven the way a person drives it.
 *
 * The first case is deliberately a survey rather than an assertion: it opens the application,
 * records what is on screen and takes a screenshot, so the selectors the real cases use come from
 * the interface itself instead of from a guess.
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ARTIFACTS = path.join(HERE, "artifacts");
/** Where the launcher points the application's settings, so a run starts from nothing. */
const SETTINGS_FILE = path.join(HERE, ".run", "config", "settings.json");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

/** The application's own page, which is the only one in the webview. */
async function openApp(): Promise<{ page: Page; close: () => Promise<void> }> {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }

  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  return { page, close: () => browser.close() };
}

/** The settings file as the application last wrote it, which is what a restart would read. */
async function writtenSettings(): Promise<{ packedTransfer?: boolean }> {
  return JSON.parse(await readFile(SETTINGS_FILE, "utf8")) as { packedTransfer?: boolean };
}

test("the window says what it is", async () => {
  await mkdir(ARTIFACTS, { recursive: true });
  const { page, close } = await openApp();
  try {
    await page.waitForLoadState("domcontentloaded");
    // The interface paints after its first call to the backend; waiting for a title is the
    // cheapest way to know it did.
    await expect(page.locator("text=7Shell").first()).toBeVisible();

    const text = await page.locator("body").innerText();
    await writeFile(path.join(ARTIFACTS, "first-screen.txt"), text, "utf8");
    await page.screenshot({ path: path.join(ARTIFACTS, "first-screen.png"), fullPage: false });

    const buttons = await page.locator("button").evaluateAll((elements) =>
      elements.map((element) => ({
        text: (element.textContent ?? "").trim(),
        label: element.getAttribute("aria-label"),
        title: element.getAttribute("title"),
      })),
    );
    await writeFile(path.join(ARTIFACTS, "buttons.json"), JSON.stringify(buttons, null, 2), "utf8");

    const fields = await page.locator("input").evaluateAll((elements) =>
      elements.map((element) => ({
        type: element.getAttribute("type"),
        placeholder: element.getAttribute("placeholder"),
        id: element.getAttribute("id"),
      })),
    );
    await writeFile(path.join(ARTIFACTS, "inputs.json"), JSON.stringify(fields, null, 2), "utf8");

    console.log(`first screen:\n${text}`);
  } finally {
    await close();
  }
});

/**
 * Creating a vault, and the shell it opens onto.
 *
 * The first thing anybody does with this application, driven the way they do it: two passwords and
 * a button. It uses the throwaway data directory the launcher set up, so it starts from nothing
 * every run — which is also why it is a real test of the first-run path rather than of whatever
 * happened to be left behind.
 */
test("a vault can be created and the shell opens", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByPlaceholder("At least 8 characters").fill("seven-shell-e2e");
    await page.getByPlaceholder("Type the password again").fill("seven-shell-e2e");
    await page.getByRole("button", { name: "Create vault" }).click();

    // The shell is up when the session list and the protocol list are on screen.
    await expect(page.getByText("Sessions", { exact: true })).toBeVisible();
    await expect(page.getByText("Protocols", { exact: true })).toBeVisible();
    await expect(page.getByText("SSH", { exact: true })).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "shell.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * The host monitor, and the switch that turns it off.
 *
 * What an end-to-end run can see of it: the panel is where it belongs, it says it is not connected
 * rather than drawing zeroes, and hiding it from the panel and hiding it from the settings are the
 * same switch — which is the part that would break silently if the two ever stopped sharing one
 * preference. The sampling itself needs a real server and is proved against the lab
 * (`crates/sevenshell-core/tests/live_monitor.rs`).
 */
test("the host monitor is on, and hiding it is one setting", async () => {
  const { page, close } = await openApp();
  try {
    const panel = page.getByRole("complementary", { name: "Host monitor" });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("No connection", { exact: false })).toBeVisible();
    // The rows are drawn with dashes rather than with zeroes — an empty column is not a reading —
    // and with no host known there is nothing to copy either.
    await expect(panel.getByRole("button", { name: "Copy the host name" })).toBeDisabled();

    // The three cards are the way in to the host's pages. With nothing connected there is no host
    // to open a page about, so they are drawn and refuse to be used rather than opening an empty
    // page — which is the state a person meets them in the first time they see the panel.
    for (const name of ["System info", "Processes", "Network"]) {
      await expect(panel.getByRole("button", { name, exact: true })).toBeDisabled();
    }

    // The panel's own button writes the preference, which is what makes the next assertion true.
    await panel.getByRole("button", { name: "Hide the monitor" }).click();
    await expect(panel).toHaveCount(0);

    await page.getByRole("button", { name: "Settings" }).click();
    const settings = page.getByRole("dialog");
    await expect(settings.getByRole("button", { name: "Hidden" })).toHaveAttribute("aria-pressed", "true");

    await settings.getByRole("button", { name: "Shown" }).click();

    // The dialog hides the rest of the window from assistive technology while it is up, so the
    // panel is asked for once it is closed: it is one preference, and both ends write it.
    await page.keyboard.press("Escape");
    await expect(settings).toHaveCount(0);
    await expect(panel).toBeVisible();
  } finally {
    await close();
  }
});

/**
 * The interface's own right-click menu, on the one surface a run without a server can reach: the
 * session list.
 *
 * Saving a session is enough to have a row; connecting is not needed and deliberately not done. What
 * is asserted is the hand-over a person would check by hand — right-click a row, see what can be done
 * to it, press Escape — and that the browser's own menu is what does *not* come up, which is the
 * other half of the same feature.
 */
test("a saved session offers its own menu on a right-click", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill("lab");
    await editor.getByPlaceholder("example.com").fill("127.0.0.1");
    await editor.getByPlaceholder("root").fill("labuser");
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // The row's name is its title as well, which is what makes it findable without connecting.
    const row = page.getByTitle("Open a terminal for lab");
    await expect(row).toBeVisible();

    await row.click({ button: "right" });
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(menu.getByRole("menuitem")).toHaveText([
      "Open a terminal for lab",
      "Open files for lab",
      "Edit session",
      "Delete session",
      "Copy address",
    ]);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "context-menu.png"), fullPage: false });

    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
  } finally {
    await close();
  }
});

/**
 * The browser's menu, which is cancelled everywhere except in a text field.
 *
 * It is asserted where it actually happens rather than by looking for a menu that is drawn by the
 * platform and appears in no screenshot: WebView2 draws its menu unless the page cancels the event,
 * so a cancelled event is the mechanism, and a field's uncancelled one is the exception that keeps
 * cut and paste working.
 */
test("the browser's own menu is cancelled, and left alone in a text field", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "New session" }).click();
    const field = page.getByPlaceholder("Production web server");
    await expect(field).toBeVisible();

    const onBody = await page.evaluate(() => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      return event.defaultPrevented;
    });

    const onField = await field.evaluate((element) => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      element.dispatchEvent(event);
      return event.defaultPrevented;
    });

    expect(onBody, "the browser's menu is not what a right-click anywhere brings up").toBe(true);
    expect(onField, "a text field keeps its own menu").toBe(false);

    await page.keyboard.press("Escape");
  } finally {
    await close();
  }
});

/**
 * The packed-transfer switch, and the file it is written to.
 *
 * The same switch also sits in the file panel's own menu, which a run without a server cannot open —
 * so what is checked here is the half that is always reachable, plus the one thing a screenshot could
 * never show: that flipping it changes what the application will do *next time*, because the choice
 * is on disk rather than in the window that made it.
 */
test("the packed-transfer switch is written down, not just remembered", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "Settings" }).click();
    const settings = page.getByRole("dialog");

    // Everything after a transfer leaves an archive on the disk, so the default is the plain one.
    await expect(settings.getByRole("button", { name: "File by file" })).toHaveAttribute("aria-pressed", "true");
    await settings.getByRole("button", { name: "One archive" }).click();
    await expect(settings.getByRole("button", { name: "One archive" })).toHaveAttribute("aria-pressed", "true");

    // Polled rather than read once: the interface applies a choice at once and writes it afterwards,
    // so what is being checked is that the file catches up rather than that it was written in the
    // same instant as the click.
    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "the choice reached the settings file",
      })
      .toBe(true);

    await settings.getByRole("button", { name: "File by file" }).click();
    await page.keyboard.press("Escape");
    await expect(settings).toHaveCount(0);

    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "and it is written back when it is turned off again",
      })
      .toBe(false);
  } finally {
    await close();
  }
});

/**
 * The clipboard, through the backend, which is the only way this build can read one.
 *
 * A script's own clipboard read never settles in WebView2 — the permission request it raises is never
 * answered, which a probe of this window showed by hanging until the test timed out — so the
 * terminal's Paste asks Rust instead, and this is that call, driven the way the interface drives it.
 *
 * It reaches past the backend facade on purpose: the facade is not on the page, and the menu entry
 * that uses this needs a server to be reachable at all. Writing first is what makes the answer a fact
 * rather than a hope about whatever happened to be on the clipboard.
 */
test("the backend can read the clipboard the terminal pastes", async () => {
  const { page, close } = await openApp();
  try {
    const answer = await page.evaluate(async () => {
      const internals = (window as unknown as { __TAURI_INTERNALS__?: { invoke: (command: string) => Promise<unknown> } })
        .__TAURI_INTERNALS__;
      if (!internals) {
        return { wrote: false, text: null };
      }
      await navigator.clipboard.writeText("seven-shell-e2e");
      return { wrote: true, text: await internals.invoke("clipboard_text") };
    });

    expect(answer.wrote, "the webview can still write a clipboard, which the copy entries rely on").toBe(true);
    expect(answer.text).toBe("seven-shell-e2e");
  } finally {
    await close();
  }
});
