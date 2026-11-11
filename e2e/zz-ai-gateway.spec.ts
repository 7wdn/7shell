/**
 * The assistant against a real OpenAI-compatible gateway.
 *
 * Environment-gated: the address, key and model are a machine's own, and a test that needed them
 * would fail everywhere else. Run it after the suite's vault case (the `zz-` name makes the full
 * suite do that) and with:
 *
 * ```text
 * SEVENSHELL_TEST_AI_BASE_URL=http://127.0.0.1:3425/v1 \
 * SEVENSHELL_TEST_AI_KEY=... SEVENSHELL_TEST_AI_MODEL=group/auto-deepseek-v4.1-flash \
 * SEVENSHELL_E2E_PORT=9100 \
 * SEVENSHELL_E2E_GREP="a vault can be created and the shell opens|assistant answers a real gateway" \
 *   pnpm app:e2e
 * ```
 *
 * The key is read from the environment, typed into the password field, and never written to this
 * repository. What this proves is the whole path: window → host → vault key → provider adapter →
 * real SSE gateway → assistant message.
 */
import { chromium, expect, test } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";
const BASE_URL = process.env.SEVENSHELL_TEST_AI_BASE_URL;
const KEY = process.env.SEVENSHELL_TEST_AI_KEY;
const MODEL = process.env.SEVENSHELL_TEST_AI_MODEL;

test("the assistant answers a real gateway", async () => {
  test.skip(!BASE_URL || !KEY || !MODEL, "needs SEVENSHELL_TEST_AI_BASE_URL / _KEY / _MODEL");
  test.setTimeout(120_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    // The suite's vault case runs first; a standalone run is skipped rather than described as a
    // failure of the gateway.
    if (
      !(await page
        .getByLabel("Search sessions")
        .isVisible()
        .catch(() => false))
    ) {
      test.skip(true, "run this with the suite's vault case so the shell is open");
    }

    const bannerSettings = page.getByRole("banner").getByRole("button", { name: "Settings" });
    const settings = page.getByRole("region", { name: "Settings" });
    if (!(await settings.isVisible().catch(() => false))) {
      await bannerSettings.click();
    }
    await expect(settings).toBeVisible();
    await settings.getByRole("button", { name: "Assistant", exact: true }).click();

    await settings.locator("[data-ai-provider-add]").click();
    const form = settings.locator("[data-ai-provider-form]");
    await expect(form).toBeVisible();

    await form.locator("#ai-provider-name").fill("Test gateway");
    // The kind changes the address default; choose it before filling the address.
    await form.locator("#ai-provider-kind").click();
    await page.getByRole("option", { name: "OpenAI", exact: true }).click();
    await form.locator("#ai-provider-base-url").fill(BASE_URL!);
    await form.locator("#ai-provider-model").fill(MODEL!);
    await form.locator("#ai-provider-key").fill(KEY!);
    await form.getByRole("button", { name: "Save provider", exact: true }).click();

    await expect(settings.getByText("Test gateway", { exact: false })).toBeVisible({ timeout: 20_000 });

    await page.locator("[data-assistant-toggle]").click();
    const composer = page.getByPlaceholder("Ask about this host…");
    await expect(composer).toBeVisible({ timeout: 15_000 });
    await expect(composer).toBeEnabled({ timeout: 15_000 });
    await composer.fill("Reply with the single word: pong");
    await composer.press("Enter");

    await expect(page.getByText("pong", { exact: true })).toBeVisible({ timeout: 90_000 });

    // The second half is the one the assistant exists for: the model asks for a real tool, the host
    // runs it through the same registry an MCP call uses, and the card reports what came back.
    await expect(composer).toBeEnabled({ timeout: 15_000 });
    await composer.fill(
      "Call the sshell_sessions_list tool with kind saved, then tell me how many saved sessions there are.",
    );
    await composer.press("Enter");

    await expect(page.getByText("sshell_sessions_list", { exact: true })).toBeVisible({ timeout: 90_000 });
    await expect(page.getByText("Done", { exact: true })).toBeVisible({ timeout: 90_000 });
  } finally {
    await browser.close();
  }
});
