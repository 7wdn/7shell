/**
 * What the running application does, driven the way a person drives it.
 *
 * The first case is deliberately a survey rather than an assertion: it opens the application,
 * records what is on screen and takes a screenshot, so the selectors the real cases use come from
 * the interface itself instead of from a guess.
 */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ARTIFACTS = path.join(HERE, "artifacts");
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
