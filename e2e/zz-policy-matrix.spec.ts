/**
 * The policy matrix through the real window and the real bridge.
 *
 * One global rule changed on the Security page must reach the next tool call without a restart:
 * `safe` is the level a read-like command lands on, and a global Deny must refuse it before the
 * session lookup or any connection. The same decision has to appear in the policy audit panel.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ENDPOINT = path.join(HERE, ".run", "config", "mcp-endpoint.json");
const BRIDGE = path.join(HERE, ".run", "bridge", process.platform === "win32" ? "sshell-mcp.exe" : "sshell-mcp");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

type Answer = {
  id?: number;
  error?: { code?: number; message?: string; data?: { code?: string } };
};

async function bridge(requests: Array<Record<string, unknown>>): Promise<Answer[]> {
  expect(existsSync(BRIDGE)).toBe(true);
  expect(existsSync(ENDPOINT)).toBe(true);
  const child = spawn(BRIDGE, ["--endpoint-file", ENDPOINT], { stdio: ["pipe", "pipe", "pipe"] });
  const answers: Answer[] = [];
  let stderr = "";
  child.stderr!.on("data", (chunk) => {
    stderr += String(chunk);
  });
  const lines = createInterface({ input: child.stdout! });
  lines.on("line", (line) => answers.push(JSON.parse(line) as Answer));
  try {
    for (const request of requests) {
      child.stdin!.write(`${JSON.stringify(request)}\n`);
    }
    await expect.poll(() => answers.length, { timeout: 20_000 }).toBe(requests.length);
  } catch (error) {
    throw new Error(`${String(error)}\nbridge stderr: ${stderr}`);
  } finally {
    child.stdin!.end();
    lines.close();
    await Promise.race([
      new Promise<void>((resolve) => child.once("exit", () => resolve())),
      new Promise<void>((resolve) => setTimeout(resolve, 5_000)),
    ]);
    if (child.exitCode === null) {
      child.kill();
    }
  }
  return answers;
}

async function openAgentAccess(page: Page): Promise<Locator> {
  const settings = page.getByRole("region", { name: "Settings" });
  if (!(await settings.isVisible().catch(() => false))) {
    await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  }
  await expect(settings).toBeVisible();
  await settings.getByRole("button", { name: "MCP", exact: true }).click();
  const toggle = settings.getByRole("switch", { name: "Allow agent access" });
  await expect(toggle).toBeEnabled({ timeout: 15_000 });
  if ((await toggle.getAttribute("aria-checked")) !== "true") {
    await toggle.click();
  }
  await expect.poll(() => existsSync(ENDPOINT), { timeout: 30_000 }).toBe(true);
  return settings;
}

async function chooseRule(page: Page, select: Locator, label: string): Promise<void> {
  await select.click();
  await page.getByRole("option", { name: label, exact: true }).click();
}

test("a global deny reaches the next tool call and the audit trail", async () => {
  test.setTimeout(120_000);
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    await expect(page.getByRole("textbox", { name: "Search sessions" })).toBeVisible({ timeout: 30_000 });
    const settings = await openAgentAccess(page);
    // The exec tool is a write, so its registry scope has to allow writes before the matrix is the
    // thing being tested; otherwise the existing read-only scope refuses it first.
    await page.locator('[data-mcp-tab="permissions"]').click();
    await page.locator('[data-mcp-scope-card="safeWrite"]').click();
    await page.locator('[data-mcp-apply="permissions"]').click();
    await expect(page.locator('[data-mcp-applied="permissions"]')).toBeVisible({ timeout: 20_000 });

    await settings
      .getByRole("navigation", { name: "Settings categories" })
      .getByRole("button", { name: "Security", exact: true })
      .click();
    await expect(page.locator("[data-policy-matrix]")).toBeVisible({ timeout: 15_000 });

    const safe = page.locator('[data-policy-layer="global"] [data-policy-rule="safe"]');
    await chooseRule(page, safe, "Deny");
    await page.locator("[data-policy-apply]").click();
    await expect(page.locator("[data-policy-notice]")).toBeVisible({ timeout: 20_000 });

    const refused = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: {
          name: "sshell_exec_run",
          arguments: { sessionId: "policy-e2e-missing", command: "true" },
        },
      },
    ]);
    expect(refused[1]?.error?.data?.code, "a denied safe command does not reach a connection").toBe("POLICY-DENIED");

    // The decision is on the policy trail, not only in the tool's error.
    await page.locator("[data-policy-audit-refresh]").click();
    await expect(page.locator("[data-policy-audit-row]").filter({ hasText: "sshell_exec_run" }).first()).toBeVisible({
      timeout: 15_000,
    });

    // Put the global rule back and take the listener down for whatever runs next.
    await chooseRule(page, safe, "Allow");
    await page.locator("[data-policy-apply]").click();
    await expect(page.locator("[data-policy-notice]")).toBeVisible({ timeout: 20_000 });

    await settings.getByRole("button", { name: "MCP", exact: true }).click();
    await page.locator('[data-mcp-tab="permissions"]').click();
    await page.locator('[data-mcp-scope-card="readOnly"]').click();
    await page.locator('[data-mcp-apply="permissions"]').click();
    await expect(page.locator('[data-mcp-applied="permissions"]')).toBeVisible({ timeout: 20_000 });
    await page.locator('[data-mcp-tab="access"]').click();
    const toggle = settings.getByRole("switch", { name: "Allow agent access" });
    if ((await toggle.getAttribute("aria-checked")) === "true") {
      await toggle.click();
    }
    await expect.poll(() => existsSync(ENDPOINT), { timeout: 10_000 }).toBe(false);
  } finally {
    await browser.close();
  }
});
