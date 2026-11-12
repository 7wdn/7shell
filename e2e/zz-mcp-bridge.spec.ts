/**
 * The MCP bridge against the real window and the real control endpoint.
 *
 * No lab, no provider key and no external MCP server: the bridge is the binary a client launches,
 * and the desktop it talks to is the one this run started. The vault case in `app.spec.ts` runs
 * first (the `zz-` name), so the shell is open when this case starts.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { chromium, expect, test } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ENDPOINT = path.join(HERE, ".run", "config", "mcp-endpoint.json");
const BRIDGE = path.join(HERE, "..", "target", "debug", process.platform === "win32" ? "sshell-mcp.exe" : "sshell-mcp");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

type Answer = {
  id?: number;
  result?: {
    protocolVersion?: string;
    tools?: unknown[];
    structuredContent?: { sessions?: unknown };
  };
  error?: { code?: number; message?: string };
};

test("the MCP bridge lists and calls the live tool table", async () => {
  test.setTimeout(120_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  try {
    await expect(page.getByRole("textbox", { name: "Search sessions" })).toBeVisible({ timeout: 30_000 });

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
    expect(existsSync(BRIDGE), `the bridge should have been built at ${BRIDGE}`).toBe(true);

    const child = spawn(BRIDGE, ["--endpoint-file", ENDPOINT], { stdio: ["pipe", "pipe", "pipe"] });
    const answers: Answer[] = [];
    let stderr = "";
    child.stderr!.on("data", (chunk) => {
      stderr += String(chunk);
    });
    const lines = createInterface({ input: child.stdout! });
    lines.on("line", (line) => answers.push(JSON.parse(line) as Answer));

    const write = (message: unknown) => child.stdin!.write(`${JSON.stringify(message)}\n`);
    write({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } });
    write({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    write({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: { name: "sshell_sessions_list", arguments: {} },
    });

    await expect.poll(() => answers.length, { timeout: 20_000 }).toBe(3);
    child.stdin!.end();
    lines.close();
    await new Promise<void>((resolve) => child.once("exit", () => resolve()));

    const answer = (id: number) => answers.find((candidate) => candidate.id === id);
    expect(answer(1)?.result?.protocolVersion, stderr).toBe("2025-06-18");
    // The live table comes from the desktop's `GET /tools`; with no external server configured it
    // is the same ten tools the bridge was built with.
    expect(answer(2)?.result?.tools?.length, stderr).toBe(10);
    expect(answer(3)?.error, stderr).toBeUndefined();
    expect(Array.isArray(answer(3)?.result?.structuredContent?.sessions), stderr).toBe(true);

    // Turning the listener off removes the file the bridge was told to read.
    if ((await toggle.getAttribute("aria-checked")) === "true") {
      await toggle.click();
    }
    await expect.poll(() => existsSync(ENDPOINT), { timeout: 10_000 }).toBe(false);
  } finally {
    await browser.close();
  }
});
