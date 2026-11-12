/**
 * The Agent access page against the real window and the real bridge.
 *
 * The three cases are the ones only a real desktop can prove: a tool switched off disappears from
 * the table a client is handed, a hidden session disappears from the list *and* is refused when a
 * caller names it anyway, and a session scope of "read only" refuses a write before a connection
 * is opened. No lab and no provider key: a saved session is enough, because every refusal here
 * happens before authentication.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ENDPOINT = path.join(HERE, ".run", "config", "mcp-endpoint.json");
const BRIDGE = path.join(HERE, "..", "target", "debug", process.platform === "win32" ? "sshell-mcp.exe" : "sshell-mcp");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

type SessionRow = { id: string; name: string };
type Answer = {
  id?: number;
  result?: {
    tools?: Array<{ name: string }>;
    structuredContent?: { sessions?: SessionRow[] };
  };
  error?: { code?: number; message?: string; data?: { code?: string } };
};

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

async function saveSession(page: Page, name: string): Promise<void> {
  await sidebar(page).getByRole("button", { name: "New session" }).click();
  const editor = page.getByRole("dialog");
  await editor.getByPlaceholder("Production web server").fill(name);
  await editor.getByPlaceholder("example.com").fill("127.0.0.1");
  await editor.getByPlaceholder("root").fill("labuser");
  await editor.locator("#session-port").fill("22");
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor).toHaveCount(0);
}

/** One bridge process; every request is written at once and every answer is collected by id. */
async function bridge(requests: Array<Record<string, unknown>>): Promise<Answer[]> {
  expect(existsSync(BRIDGE), `the bridge should have been built at ${BRIDGE}`).toBe(true);
  expect(existsSync(ENDPOINT), `the listener should have written ${ENDPOINT}`).toBe(true);

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

function answer(answers: Answer[], id: number): Answer | undefined {
  return answers.find((candidate) => candidate.id === id);
}

async function openAgentAccess(page: Page): Promise<Locator> {
  const settings = page.getByRole("region", { name: "Settings" });
  if (!(await settings.isVisible().catch(() => false))) {
    await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  }
  await expect(settings).toBeVisible();
  await settings.getByRole("button", { name: "Agent access", exact: true }).click();
  await expect(page.locator('[data-mcp-tab="access"]')).toBeVisible();
  return settings;
}

async function enableListener(page: Page, settings: Locator): Promise<void> {
  const toggle = settings.getByRole("switch", { name: "Allow agent access" });
  await expect(toggle).toBeEnabled({ timeout: 15_000 });
  if ((await toggle.getAttribute("aria-checked")) !== "true") {
    await toggle.click();
  }
  await expect.poll(() => existsSync(ENDPOINT), { timeout: 30_000 }).toBe(true);
}

async function applyCapabilities(page: Page): Promise<void> {
  await page.locator('[data-mcp-apply="capabilities"]').click();
  await expect(page.locator('[data-mcp-applied="capabilities"]')).toBeVisible({ timeout: 20_000 });
}

async function applyPermissions(page: Page): Promise<void> {
  await page.locator('[data-mcp-apply="permissions"]').click();
  await expect(page.locator('[data-mcp-applied="permissions"]')).toBeVisible({ timeout: 20_000 });
}

test("agent access controls the live tool table and the sessions it can reach", async () => {
  test.setTimeout(180_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    await expect(page.getByLabel("Search sessions")).toBeVisible({ timeout: 30_000 });
    const name = `governance-${Date.now()}`;
    await saveSession(page, name);
    const settings = await openAgentAccess(page);
    await enableListener(page, settings);

    // A previous run may have left a custom visibility allowlist. Start from "every session" so a
    // newly saved one is in the table; this is also the page's own reset control.
    await page.locator('[data-mcp-tab="permissions"]').click();
    if (
      await page
        .locator("[data-mcp-allow-all]")
        .isVisible()
        .catch(() => false)
    ) {
      await page.locator("[data-mcp-allow-all]").click();
      await applyPermissions(page);
    }

    // The bridge sees the session before anything is hidden; its id is what a hidden call names.
    const before = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "sshell_sessions_list", arguments: {} } },
    ]);
    const sessions = answer(before, 2)?.result?.structuredContent?.sessions ?? [];
    const saved = sessions.find((session) => session.name === name);
    expect(saved, "the saved session is in the live list").toBeDefined();
    const sessionId = saved!.id;

    // 1. Hide the session in the permissions tree.
    await page.locator('[data-mcp-tab="permissions"]').click();
    const row = page.locator('[data-mcp-permission-row][data-mcp-permission-kind="session"]').filter({ hasText: name });
    await expect(row).toBeVisible({ timeout: 15_000 });
    const visibility = row.locator("[data-mcp-visibility]");
    await expect(visibility).toBeChecked();
    await visibility.uncheck();
    await expect(row.locator("[data-mcp-hidden]")).toBeVisible();
    await applyPermissions(page);

    const hidden = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "sshell_sessions_list", arguments: {} } },
      {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "sshell_sftp_read_text", arguments: { sessionId, path: "/tmp/governance.txt" } },
      },
    ]);
    const hiddenSessions = answer(hidden, 2)?.result?.structuredContent?.sessions ?? [];
    expect(hiddenSessions.some((session) => session.id === sessionId)).toBe(false);
    expect(answer(hidden, 3)?.error?.data?.code, "a named hidden session is refused").toBe("MCP-SESSION-HIDDEN");

    // 2. Put the session back and narrow the global scope to read-only. A write is refused before
    //    any connection is opened, so no lab is involved.
    await visibility.check();
    await page.locator('[data-mcp-scope-card="readOnly"]').click();
    await applyPermissions(page);
    await expect.poll(() => existsSync(ENDPOINT), { timeout: 30_000 }).toBe(true);

    const readOnly = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "sshell_sftp_write_text", arguments: { sessionId, path: "/tmp/governance.txt", text: "x" } },
      },
    ]);
    expect(answer(readOnly, 2)?.error?.data?.code, "a write tool is refused under read-only").toBe("POLICY-DENIED");

    // 3. Switch one tool off and ask the bridge again.
    await page.locator('[data-mcp-scope-card="safeWrite"]').click();
    await applyPermissions(page);
    await page.locator('[data-mcp-tab="capabilities"]').click();
    const tool = page.locator('[data-mcp-tool][data-mcp-tool-name="sshell_exec_run"]');
    await expect(tool).toBeVisible({ timeout: 15_000 });
    await tool.locator("[data-mcp-tool-switch]").click();
    await applyCapabilities(page);

    const disabled = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
      {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "sshell_exec_run", arguments: { sessionId, command: "true" } },
      },
    ]);
    const names = (answer(disabled, 2)?.result?.tools ?? []).map((entry) => entry.name);
    expect(names).not.toContain("sshell_exec_run");
    expect(answer(disabled, 3)?.error?.data?.code, "a switched-off tool is refused by name").toBe("MCP-TOOL-DISABLED");

    // 4. Restore the page and the listener for whatever runs after this case.
    await tool.locator("[data-mcp-tool-switch]").click();
    await applyCapabilities(page);
    await page.locator('[data-mcp-tab="permissions"]').click();
    if (
      await page
        .locator("[data-mcp-allow-all]")
        .isVisible()
        .catch(() => false)
    ) {
      await page.locator("[data-mcp-allow-all]").click();
    }
    await page.locator('[data-mcp-scope-card="safeWrite"]').click();
    await applyPermissions(page);

    const restored = await bridge([
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } },
      { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
    ]);
    expect((answer(restored, 2)?.result?.tools ?? []).length).toBeGreaterThanOrEqual(10);
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
