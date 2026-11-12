/**
 * Server content — resources and prompts — from a real stdio MCP server in the real window.
 *
 * The fixture beside this file (`fixtures/mcp-content-server.mjs`) is a child process of the
 * application, not a stub inside it: it is started by `node` on PATH, handshakes over a pipe, and
 * keeps answering while the picker is open. What only this case can prove is the whole chain at
 * once — the settings page saves a configuration, the supervisor spawns the program, the cache is
 * filled from what that program answered, the picker draws it, and the two things a person does
 * with it (attach a resource, insert a rendered prompt) reach the composer the way the contract
 * says they do.
 *
 * No lab, no provider key and no network: the fixture serves fixed text from disk. It runs after
 * the suite's vault case — the `zz-` name puts it there — and skips only when the shell is not
 * open, never because a half of it is missing.
 */
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE = path.join(HERE, "fixtures", "mcp-content-server.mjs");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

/** The identifier the row and every frozen `data-mcp-*` selector below is keyed by. */
const SERVER_ID = "content-e2e";
const SERVER_NAME = "Content fixture";
/** A second name, so the provider row and the server row can never be confused for one another. */
const PROVIDER_NAME = "Local content model";
const PROVIDER_MODEL = "fixture-model";
const PROVIDER_URL = "http://127.0.0.1:11434/v1";
/** What the fixture serves, verbatim, on both sides of the read. */
const RESOURCE_URI = "file:///welcome.txt";
const RESOURCE_TEXT = "Hello from the fixture.";
const PROMPT_TOPIC = "networks";

/** The settings region, opened if it is not on screen already. */
async function openSettings(page: Page): Promise<Locator> {
  const settings = page.getByRole("region", { name: "Settings" });
  if (!(await settings.isVisible().catch(() => false))) {
    await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  }
  await expect(settings).toBeVisible({ timeout: 15_000 });
  return settings;
}

/** The assistant's composer, with the panel opened if it is not on screen already. */
async function openAssistant(page: Page): Promise<Locator> {
  const composer = page.getByPlaceholder("Ask about this host…");
  if (!(await composer.isVisible().catch(() => false))) {
    await page.locator("[data-assistant-toggle]").click();
  }
  await expect(composer).toBeVisible({ timeout: 15_000 });
  return composer;
}

/**
 * Adds the fixture as an external server and waits until the host says it is talking to it.
 *
 * "Ready" is the row's own answer, not the form's: the configuration being saved and the program
 * having answered a handshake are different facts, and only the second one means the picker will
 * have anything to draw.
 */
async function addContentServer(page: Page, settings: Locator): Promise<void> {
  await settings.getByRole("button", { name: "MCP", exact: true }).click();
  await expect(settings.locator("[data-mcp-servers]")).toBeVisible({ timeout: 15_000 });

  // A leftover from a run that failed before its cleanup would collide on the identifier.
  const existing = settings.locator(`[data-mcp-server-row][data-mcp-server-id="${SERVER_ID}"]`);
  if ((await existing.count()) > 0) {
    await removeContentServer(page, settings);
  }

  await settings.locator("[data-mcp-server-add]").click();
  const form = settings.locator("[data-mcp-server-form]");
  await expect(form).toBeVisible();
  await form.locator("#mcp-server-id").fill(SERVER_ID);
  await form.locator("#mcp-server-name").fill(SERVER_NAME);
  await form.locator("#mcp-server-command").fill("node");
  // One argument per line; the fixture's own absolute path is the only one there is.
  await form.locator("#mcp-server-args").fill(FIXTURE);
  await form.locator("[data-mcp-server-save]").click();

  const row = settings.locator(`[data-mcp-server-row][data-mcp-server-id="${SERVER_ID}"]`);
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row.getByText("Ready", { exact: true })).toBeVisible({ timeout: 30_000 });
}

/** Removes the server through the same page that added it, confirmation dialog and all. */
async function removeContentServer(page: Page, settings: Locator): Promise<void> {
  const row = settings.locator(`[data-mcp-server-row][data-mcp-server-id="${SERVER_ID}"]`);
  if ((await row.count()) === 0) {
    return;
  }
  await row.locator("[data-mcp-server-remove]").click();
  await page.getByRole("dialog").getByRole("button", { name: "Remove", exact: true }).click();
  await expect(row).toHaveCount(0, { timeout: 15_000 });
}

/**
 * A local provider, drawn only when the panel has none.
 *
 * The composer is where an inserted prompt lands, and a panel with no provider at all replaces it
 * with a setup card. A local one needs no key and is never sent to, so it costs nothing to leave
 * behind for a moment — it is removed again in the cleanup below.
 */
async function addLocalProvider(page: Page): Promise<void> {
  const settings = await openSettings(page);
  await settings.getByRole("button", { name: "AI", exact: true }).click();
  await settings.locator("[data-ai-provider-add]").click();

  const form = settings.locator("[data-ai-provider-form]");
  await expect(form).toBeVisible();
  await form.locator("#ai-provider-name").fill(PROVIDER_NAME);
  // The kind rewrites the address only while it still holds the previous kind's default, so it is
  // chosen before the address is typed.
  await form.locator("#ai-provider-kind").click();
  await page.getByRole("option", { name: "Local (OpenAI-compatible)", exact: true }).click();
  await form.locator("#ai-provider-base-url").fill(PROVIDER_URL);
  await form.locator("#ai-provider-model").fill(PROVIDER_MODEL);
  await form.getByRole("button", { name: "Save provider", exact: true }).click();

  await expect(settings.getByText(PROVIDER_NAME, { exact: false })).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Close settings" }).click();
}

/** Removes every provider row this case could have added. Best-effort: cleanup never fails the case. */
async function removeLocalProviders(page: Page): Promise<void> {
  const settings = await openSettings(page);
  await settings.getByRole("button", { name: "AI", exact: true }).click();

  for (let attempt = 0; attempt < 3; attempt += 1) {
    const row = settings.locator("li").filter({ hasText: PROVIDER_NAME }).first();
    if ((await row.count()) === 0) {
      return;
    }
    await row.getByRole("button", { name: "Remove", exact: true }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Remove", exact: true }).click();
    await expect(row).toHaveCount(0, { timeout: 15_000 });
  }
}

/**
 * Closes the picker the way any dialog is left when its own close control has no frozen selector:
 * Escape first, and a button labelled "Close" only if the dialog is still there afterwards.
 */
async function closeContentDialog(page: Page, dialog: Locator): Promise<void> {
  await page.keyboard.press("Escape");
  if (await dialog.isVisible().catch(() => false)) {
    const close = dialog.getByRole("button", { name: "Close", exact: true });
    if ((await close.count()) > 0) {
      await close.first().click();
    }
  }
  await expect(dialog).toBeHidden({ timeout: 10_000 });
}

test("server content reads, attaches and inserts from a real stdio server", async () => {
  test.setTimeout(180_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  // The suite's vault case runs first; a standalone run is skipped rather than described as a
  // failure of the picker.
  if (
    !(await page
      .getByRole("textbox", { name: "Search sessions" })
      .isVisible()
      .catch(() => false))
  ) {
    await browser.close();
    test.skip(true, "run this with the suite's vault case so the shell is open");
  }

  try {
    expect(existsSync(FIXTURE), `the fixture should be at ${FIXTURE}`).toBe(true);

    // -- The server the picker reads ----------------------------------------
    const settings = await openSettings(page);
    await addContentServer(page, settings);
    await page.getByRole("button", { name: "Close settings" }).click();

    // -- The composer an insert lands in ------------------------------------
    const composer = await openAssistant(page);
    const setup = page.locator("[data-assistant-setup]");
    if (await setup.isVisible().catch(() => false)) {
      await addLocalProvider(page);
      // A provider that was already pinned and has no key would keep the panel blocked, so the
      // local one is chosen in the panel when the card is still there.
      if (await setup.isVisible().catch(() => false)) {
        const chooser = page.locator("[data-assistant-panel]").getByRole("combobox", { name: "Provider" });
        await chooser.click();
        await page.getByRole("option", { name: PROVIDER_NAME, exact: false }).first().click();
      }
      await expect(setup).toBeHidden({ timeout: 15_000 });
    }

    const dialog = page.locator("[data-mcp-content-dialog]");

    // -- Resources: read it, see it, attach it ------------------------------
    await page.locator("[data-assistant-content]").click();
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await dialog.locator('[data-mcp-content-tab="resources"]').click();

    const resource = dialog.locator(`[data-mcp-resource="${RESOURCE_URI}"][data-mcp-resource-server="${SERVER_ID}"]`);
    await expect(resource).toBeVisible({ timeout: 20_000 });

    await resource.locator("[data-mcp-resource-read]").click();
    const preview = dialog.locator("[data-mcp-content-preview]").first();
    await expect(preview).toContainText(RESOURCE_TEXT, { timeout: 20_000 });

    await resource.locator("[data-mcp-resource-attach]").click();
    await closeContentDialog(page, dialog);

    // The chip is what the next question would carry, so it is on screen without the dialog.
    const chip = page.locator("[data-assistant-attachment]").first();
    await expect(chip).toBeVisible({ timeout: 10_000 });
    await expect(chip).toContainText("welcome");

    // -- Prompts: fill the argument, render, insert -------------------------
    await page.locator("[data-assistant-content]").click();
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await dialog.locator('[data-mcp-content-tab="prompts"]').click();

    const prompt = dialog.locator(`[data-mcp-prompt="${SERVER_ID}:summarize"]`);
    await expect(prompt).toBeVisible({ timeout: 20_000 });
    await prompt.click();
    await dialog.locator('[data-mcp-prompt-argument="topic"]').fill(PROMPT_TOPIC);
    await dialog.locator("[data-mcp-prompt-render]").click();
    await expect(dialog.locator("[data-mcp-content-preview]").first()).toContainText(`Summarize ${PROMPT_TOPIC}`, {
      timeout: 20_000,
    });

    await dialog.locator("[data-mcp-prompt-insert]").click();
    await closeContentDialog(page, dialog);

    // The rendered text is in the line a person edits, and nothing was sent: the panel is idle.
    await expect(composer).toHaveValue(new RegExp(`Summarize ${PROMPT_TOPIC}`));
    await expect(composer).toBeEnabled();
  } finally {
    // Best-effort on purpose: this runs in a failing case too, and a window that cannot answer must
    // not replace the failure above with one from the cleanup.
    try {
      // A picker left open would swallow the clicks that reach the settings page.
      await page.keyboard.press("Escape");
      const settings = await openSettings(page);
      await settings.getByRole("button", { name: "MCP", exact: true }).click();
      await removeContentServer(page, settings);
      await removeLocalProviders(page);
      await page.getByRole("button", { name: "Close settings" }).click();
    } catch {
      // The gate may be up, or the window may be gone; the case's own failure is the one to report.
    }
    await browser.close();
  }
});
