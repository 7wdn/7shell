/** Real terminal output, persistent processes and DOM identity distinguish a split from a fresh screen. */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";
const LAB_HOST = process.env.SEVENSHELL_TEST_HOST;
const LAB_USER = process.env.SEVENSHELL_TEST_USER;
const LAB_PASSWORD = process.env.SEVENSHELL_TEST_PASSWORD;
const ARTIFACTS = path.join(path.dirname(fileURLToPath(import.meta.url)), "artifacts");

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

function panes(page: Page): Locator {
  return page.locator("[data-terminal-pane]").filter({ visible: true });
}

function pane(page: Page, id: string): Locator {
  return page.locator(`[data-terminal-pane][data-tab-id="${id}"]`);
}

async function window(): Promise<{ page: Page; close: () => Promise<void> }> {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) throw new Error("the application has no webview context");
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  const gate = page.getByRole("heading", { name: "Create the vault" });
  await expect(gate.or(page.locator("[data-session-list]"))).toBeVisible({ timeout: 60_000 });
  if (await gate.isVisible()) {
    await page.getByRole("textbox", { name: "Password", exact: true }).fill("terminal-split-test-password");
    await page.getByRole("textbox", { name: "Confirm password" }).fill("terminal-split-test-password");
    await page.getByRole("button", { name: "Create vault" }).click();
    await expect(page.locator("[data-session-list]")).toBeVisible({ timeout: 60_000 });
  }
  return {
    page,
    close: async () => {
      try {
        const tabs = page.locator("[data-workspace-tabs] [data-tab]");
        for (let guard = 0; guard < 12 && (await tabs.count()) > 0; guard += 1) {
          const count = await tabs.count();
          await tabs.last().locator("button").last().click();
          await expect(tabs).toHaveCount(count - 1);
        }
        await page.reload({ waitUntil: "domcontentloaded" });
        await expect(page.locator("[data-session-list]")).toBeVisible({ timeout: 60_000 });
      } finally {
        await browser.close();
      }
    },
  };
}

async function saveLocal(page: Page, name: string): Promise<void> {
  await sidebar(page).getByRole("button", { name: "New session" }).click();
  const editor = page.getByRole("dialog");
  await editor.getByPlaceholder("Production web server").fill(name);
  await editor.locator("#session-protocol").click();
  await page.getByRole("listbox").getByRole("option", { name: "Local terminal", exact: true }).click();
  await expect(editor.locator("#session-local-shell")).toBeVisible();
  await editor.getByRole("button", { name: "Save", exact: true }).click();
  await expect(editor).toHaveCount(0);
}

async function openSaved(page: Page, name: string): Promise<Locator> {
  await sidebar(page).getByTitle(`Open a terminal for ${name}`, { exact: true }).click();
  const terminal = page.getByRole("group", { name: `Terminal: ${name}`, exact: true });
  await expect(terminal).toBeVisible({ timeout: 30_000 });
  await expect(terminal.locator(".xterm-rows")).toContainText(/[>$#]/, { timeout: 30_000 });
  return terminal;
}

async function send(terminal: Locator, command: string): Promise<void> {
  const field = terminal.getByLabel("Command input", { exact: true });
  await field.fill(command);
  await expect(field).toBeFocused();
  await field.press("Enter");
}

async function moveExisting(
  page: Page,
  target: Locator,
  sourceId: string,
  direction: "right" | "down" = "right",
): Promise<void> {
  await target.locator("[data-split-picker-open]").click();
  const picker = page.locator("[data-split-picker]");
  await expect(picker).toBeVisible();
  await picker.locator('[data-split-source="existing"]').click();
  await picker.locator(`[data-split-direction="${direction}"]`).click();
  await picker.locator(`[data-split-existing-row][data-pane-id="${sourceId}"]`).click();
  await expect(picker).toHaveCount(0);
}

async function counter(terminal: Locator, marker: string): Promise<number> {
  const text = (await terminal.locator(".xterm-rows").textContent()) ?? "";
  return Math.max(0, ...[...text.matchAll(new RegExp(`${marker}:(\\d+)`, "g"))].map((match) => Number(match[1])));
}

test("terminal panes keep running through nested splits, moves, zoom and individual closes", async () => {
  const { page, close } = await window();
  const stamp = Date.now() % 100000;
  const nameA = `split-A-${stamp}`;
  const nameB = `split-B-${stamp}`;
  const history = `history-before-split-${stamp}`;
  const marker = `persistent-pane-${stamp}`;
  let running: Locator | undefined;
  try {
    await saveLocal(page, nameA);
    const a = await openSaved(page, nameA);
    const idA = (await a.getAttribute("data-tab-id")) as string;
    const handleA = await a.getAttribute("data-handle");
    const originalScreen = await a.locator(".xterm").elementHandle();
    await send(a, `echo ${history}`);
    await expect(a.locator(".xterm-rows")).toContainText(history);
    await a.locator(".xterm").click();
    await page.keyboard.type(`node -e "let n=0;setInterval(()=>console.log('${marker}:'+ ++n),250)"`);
    await page.keyboard.press("Enter");
    running = a;
    await expect.poll(() => counter(a, marker)).toBeGreaterThan(2);
    const beforeMove = await counter(a, marker);

    await saveLocal(page, nameB);
    const b = await openSaved(page, nameB);
    await moveExisting(page, b, idA);
    await expect(page.locator("[data-workspace-tabs] [data-tab]")).toHaveCount(1);
    await expect(panes(page)).toHaveCount(2);
    await expect(a).toHaveAttribute("data-handle", handleA as string);
    expect(await originalScreen?.evaluate((element) => element.isConnected)).toBe(true);
    await expect.poll(() => counter(a, marker)).toBeGreaterThan(beforeMove);

    await send(b, `echo only-B-${stamp}`);
    await expect(b.locator(".xterm-rows")).toContainText(`only-B-${stamp}`);
    await expect(a.locator(".xterm-rows")).not.toContainText(`only-B-${stamp}`);
    await expect(b).toHaveAttribute("data-active", "true");
    await b.getByLabel("Command input", { exact: true }).press("Control+Shift+]");
    await expect(a).toHaveAttribute("data-active", "true");

    // Splitting the left pane down makes a genuine nested layout beside the existing right pane.
    await b.getByLabel("Command input", { exact: true }).click();
    await page.keyboard.press("Control+Shift+O");
    await expect(panes(page)).toHaveCount(3);
    const idC = (await page.locator('[data-terminal-pane][data-active="true"]').getAttribute("data-tab-id")) as string;
    const third = pane(page, idC);
    await expect(third.locator(".xterm-rows")).toContainText(/[>$#]/, { timeout: 30_000 });
    await send(third, `echo only-C-${stamp}`);
    await expect(third.locator(".xterm-rows")).toContainText(`only-C-${stamp}`);
    await expect(b.locator(".xterm-rows")).not.toContainText(`only-C-${stamp}`);
    await expect(page.locator("[data-terminal-divider]")).toHaveCount(2);
    await expect(page.locator('[data-terminal-divider][aria-orientation="vertical"]')).toHaveCount(1);
    await expect(page.locator('[data-terminal-divider][aria-orientation="horizontal"]')).toHaveCount(1);

    const divider = page.locator('[data-terminal-divider][aria-orientation="vertical"]');
    const bounds = await divider.boundingBox();
    const widthBefore = (await b.boundingBox())!.width;
    if (!bounds) throw new Error("the terminal divider has no visible bounds");
    await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width / 2 + 40, bounds.y + bounds.height / 2);
    await page.mouse.up();
    await expect.poll(async () => (await b.boundingBox())!.width).not.toBe(widthBefore);

    await a.locator("[data-pane-zoom]").click();
    await expect(panes(page)).toHaveCount(1);
    await expect(page.locator("[data-terminal-divider]")).toHaveCount(0);
    await a.locator("[data-pane-zoom]").click();
    await expect(panes(page)).toHaveCount(3);
    await a.locator("[data-pane-zoom]").click();
    await a.locator("[data-pane-detach]").click();
    await expect(page.locator("[data-workspace-tabs] [data-tab]")).toHaveCount(2);
    await page.locator("[data-workspace-tabs] [data-tab]").first().locator("button").first().click();
    await moveExisting(page, b, idA);
    await expect(panes(page)).toHaveCount(3);
    await expect(a).toHaveAttribute("data-handle", handleA as string);
    expect(await originalScreen?.evaluate((element) => element.isConnected)).toBe(true);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "terminal-splits.png") });
    const beforeClose = await counter(a, marker);
    await third.getByLabel("Command input", { exact: true }).press("Control+Shift+X");
    await expect(panes(page)).toHaveCount(2);
    await expect.poll(() => counter(a, marker)).toBeGreaterThan(beforeClose);

    // Search scrollback created before every layout change, after enough output to scroll it away.
    await a.locator("[data-pane-zoom]").click();
    await a.getByRole("textbox", { name: "Terminal input", exact: true }).focus();
    await page.keyboard.press("Control+f");
    await a.getByLabel("Find in terminal", { exact: true }).fill(history);
    await expect(a.locator("span.min-w-10")).toContainText(/\d+\/[1-9]\d*/);
    await a.getByRole("button", { name: "Close the search", exact: true }).click();
  } finally {
    if (running && (await running.isVisible())) {
      await running
        .getByRole("textbox", { name: "Terminal input", exact: true })
        .focus()
        .catch(() => {});
      await page.keyboard.press("Control+c").catch(() => {});
    }
    await close();
  }
});

test("terminal panes mix local and SSH sessions with independent input and file panels", async () => {
  test.skip(!LAB_HOST || !LAB_USER || !LAB_PASSWORD, "the SSH lab is not configured");
  const { page, close } = await window();
  const stamp = Date.now() % 100000;
  const localName = `mixed-local-${stamp}`;
  const sshName = `mixed-ssh-${stamp}`;
  try {
    await saveLocal(page, localName);
    const local = await openSaved(page, localName);
    const localId = (await local.getAttribute("data-tab-id")) as string;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(sshName);
    await editor.getByPlaceholder("example.com").fill(LAB_HOST as string);
    await editor.locator("#session-port").fill(process.env.SEVENSHELL_TEST_PORT ?? "22");
    await editor.getByPlaceholder("root").fill(LAB_USER as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await editor.getByRole("button", { name: "Save", exact: true }).click();
    await expect(editor).toHaveCount(0);
    await local.locator("[data-split-picker-open]").click();
    const picker = page.locator("[data-split-picker]");
    await picker.locator('[data-split-source="new"]').click();
    await picker.getByRole("textbox", { name: "Search terminals or sessions" }).fill(sshName);
    await picker.locator("[data-split-new-row]").click();
    const ssh = page.getByRole("group", { name: `Terminal: ${sshName}`, exact: true });
    const trust = page.getByRole("button", { name: "Trust this key and connect", exact: true });
    await expect(trust.or(ssh)).toBeVisible({ timeout: 30_000 });
    if (await trust.isVisible()) await trust.click();
    await expect(ssh).toBeVisible({ timeout: 30_000 });
    await expect(ssh.getByRole("navigation", { name: "Path" })).toBeVisible({ timeout: 30_000 });
    await expect(panes(page)).toHaveCount(2);
    await send(ssh, `printf 'ssh-only-${stamp}\\n'`);
    await expect(ssh.locator(".xterm-rows")).toContainText(`ssh-only-${stamp}`);
    await send(ssh, "cd /tmp");
    await expect(ssh.getByRole("navigation", { name: "Path" })).toContainText("tmp");
    await expect(local.locator(".xterm-rows")).not.toContainText(`ssh-only-${stamp}`);
    await ssh.getByLabel("Command input", { exact: true }).click();
    await page.keyboard.press("Control+Shift+O");
    await expect(panes(page)).toHaveCount(3);
    const peerId = (await page
      .locator('[data-terminal-pane][data-active="true"]')
      .getAttribute("data-tab-id")) as string;
    const peer = pane(page, peerId);
    await expect(peer.getByRole("navigation", { name: "Path" })).toBeVisible({ timeout: 30_000 });
    const stripBounds = await peer.locator("[data-terminal-strip]").boundingBox();
    const panelBounds = await peer.locator("[data-terminal-panel]").boundingBox();
    expect(stripBounds).not.toBeNull();
    expect(panelBounds).not.toBeNull();
    expect(stripBounds!.y + stripBounds!.height).toBeLessThanOrEqual(panelBounds!.y + 1);
    await page.locator("[data-broadcast-open]").click();
    await page.locator(`[data-broadcast-target][data-pane-id="${localId}"]`).uncheck();
    await page.locator("[data-broadcast-start]").click();
    await ssh.getByRole("textbox", { name: "Terminal input", exact: true }).focus();
    await page.keyboard.type("cd /var");
    await page.keyboard.press("Enter");
    for (const target of [ssh, peer])
      await expect(target.getByRole("navigation", { name: "Path" })).toContainText("var");
    await send(ssh, `echo ssh-broadcast-${stamp}`);
    for (const target of [ssh, peer])
      await expect(target.locator(".xterm-rows")).toContainText(`ssh-broadcast-${stamp}`);
    await expect(local.locator(".xterm-rows")).not.toContainText(`ssh-broadcast-${stamp}`);
    await page.locator("[data-broadcast-stop]").click();
    await send(local, `echo local-only-${stamp}`);
    await expect(local.locator(".xterm-rows")).toContainText(`local-only-${stamp}`);
    await expect(ssh.locator(".xterm-rows")).not.toContainText(`local-only-${stamp}`);
    await ssh.locator("[data-pane-close]").click();
    await expect(panes(page)).toHaveCount(2);
    await send(peer, `printf 'peer-alive-${stamp}\\n'`);
    await expect(peer.locator(".xterm-rows")).toContainText(`peer-alive-${stamp}`);
    await send(peer, "cd /tmp");
    await expect(peer.getByRole("navigation", { name: "Path" })).toContainText("tmp");
    await peer.locator("[data-pane-close]").click();
    await expect(panes(page)).toHaveCount(1);
    await expect(pane(page, localId)).toBeVisible();
    await send(local, `echo still-running-${stamp}`);
    await expect(local.locator(".xterm-rows")).toContainText(`still-running-${stamp}`);
  } catch (error) {
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "terminal-splits-ssh-failure.png") });
    const state = await panes(page).evaluateAll((elements) =>
      elements.map((element) => ({
        paneId: element.getAttribute("data-tab-id"),
        status: element.getAttribute("data-status"),
        footer: [...element.querySelectorAll("footer")].map((footer) => footer.textContent?.trim()),
      })),
    );
    await writeFile(path.join(ARTIFACTS, "terminal-splits-ssh-state.json"), JSON.stringify(state, null, 2));
    await test.info().attach("pane-close-state", {
      body: Buffer.from(JSON.stringify(state, null, 2)),
      contentType: "application/json",
    });
    throw error;
  } finally {
    await close();
  }
});

test("terminal broadcast sends keys, paste and commands only to the chosen panes", async () => {
  const { page, close } = await window();
  const stamp = Date.now() % 100000;
  try {
    await saveLocal(page, `broadcast-${stamp}`);
    const original = await openSaved(page, `broadcast-${stamp}`);
    const idA = (await original.getAttribute("data-tab-id")) as string;
    const a = pane(page, idA);
    await a.locator("[data-split-right]").click();
    await expect(panes(page)).toHaveCount(2);
    const idB = (await page.locator('[data-terminal-pane][data-active="true"]').getAttribute("data-tab-id")) as string;
    const b = pane(page, idB);
    await expect(b.locator(".xterm-rows")).toContainText(/[>$#]/, { timeout: 30_000 });
    await b.locator("[data-split-down]").click();
    await expect(panes(page)).toHaveCount(3);
    const idC = (await page.locator('[data-terminal-pane][data-active="true"]').getAttribute("data-tab-id")) as string;
    const c = pane(page, idC);
    await expect(c.locator(".xterm-rows")).toContainText(/[>$#]/, { timeout: 30_000 });
    await expect(page.locator("[data-terminal-broadcast]")).toHaveAttribute("data-enabled", "false");

    await send(a, `echo default-only-${stamp}`);
    await expect(a.locator(".xterm-rows")).toContainText(`default-only-${stamp}`);
    await expect(b.locator(".xterm-rows")).not.toContainText(`default-only-${stamp}`);

    await page.locator("[data-broadcast-open]").click();
    const picker = page.locator("[data-broadcast-picker]");
    await expect(picker).toBeVisible();
    await picker.locator(`[data-broadcast-target][data-pane-id="${idC}"]`).uncheck();
    await picker.locator("[data-broadcast-start]").click();
    await expect(picker).toHaveCount(0);
    await expect(a).toHaveAttribute("data-broadcast", "true");
    await expect(b).toHaveAttribute("data-broadcast", "true");
    await expect(c).toHaveAttribute("data-broadcast", "false");

    await a.getByRole("textbox", { name: "Terminal input", exact: true }).focus();
    await page.keyboard.type(`echo key-shared-${stamp}`);
    await page.keyboard.press("Enter");
    for (const target of [a, b]) await expect(target.locator(".xterm-rows")).toContainText(`key-shared-${stamp}`);
    await expect(c.locator(".xterm-rows")).not.toContainText(`key-shared-${stamp}`);

    await a.getByRole("textbox", { name: "Terminal input", exact: true }).evaluate((element, marker) => {
      const clipboard = new DataTransfer();
      clipboard.setData("text/plain", `echo ${marker}`);
      element.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, clipboardData: clipboard }));
    }, `paste-shared-${stamp}`);
    await page.keyboard.press("Enter");
    for (const target of [a, b]) await expect(target.locator(".xterm-rows")).toContainText(`paste-shared-${stamp}`);
    await expect(c.locator(".xterm-rows")).not.toContainText(`paste-shared-${stamp}`);

    await page.keyboard.insertText(`echo 中文-shared-${stamp}`);
    await page.keyboard.press("Enter");
    for (const target of [a, b]) await expect(target.locator(".xterm-rows")).toContainText(`中文-shared-${stamp}`);
    await expect(c.locator(".xterm-rows")).not.toContainText(`中文-shared-${stamp}`);

    await send(c, `echo excluded-origin-${stamp}`);
    await expect(c.locator(".xterm-rows")).toContainText(`excluded-origin-${stamp}`);
    await expect(a.locator(".xterm-rows")).not.toContainText(`excluded-origin-${stamp}`);
    await send(a, `echo command-shared-${stamp}`);
    for (const target of [a, b]) await expect(target.locator(".xterm-rows")).toContainText(`command-shared-${stamp}`);

    const field = a.getByLabel("Command input", { exact: true });
    await field.fill(`rm -rf /tmp/broadcast-review-${stamp}`);
    await field.press("Enter");
    const approval = page.getByRole("dialog");
    await expect(approval.getByText("Send this command to the broadcast panes", { exact: true })).toBeVisible();
    for (const target of [a, b])
      await expect(target.locator(".xterm-rows")).not.toContainText(`broadcast-review-${stamp}`);
    await approval.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(field).toHaveValue(`rm -rf /tmp/broadcast-review-${stamp}`);
    await field.fill("");

    await a.getByRole("textbox", { name: "Terminal input", exact: true }).focus();
    const tick = `broadcast-tick-${stamp}`;
    await page.keyboard.type(`node -e "let n=0;setInterval(()=>console.log('${tick}:'+ ++n),500)"`);
    await page.keyboard.press("Enter");
    for (const target of [a, b]) await expect.poll(() => counter(target, tick)).toBeGreaterThan(2);
    await page.keyboard.press("Control+c");
    await page.keyboard.type(`echo interrupted-both-${stamp}`);
    await page.keyboard.press("Enter");
    for (const target of [a, b]) await expect(target.locator(".xterm-rows")).toContainText(`interrupted-both-${stamp}`);
    const stopped = await Promise.all([counter(a, tick), counter(b, tick)]);
    // Echo alone can be terminal input echo even if the process is still running. Missed Ctrl+C
    // would emit more ticks within this interval, proving the shared interrupt did not arrive.
    await page.waitForTimeout(1_500);
    expect(await Promise.all([counter(a, tick), counter(b, tick)])).toEqual(stopped);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "terminal-broadcast.png") });
    await page.locator("[data-broadcast-stop]").click();
    await send(a, `echo stopped-only-${stamp}`);
    await expect(a.locator(".xterm-rows")).toContainText(`stopped-only-${stamp}`);
    await expect(b.locator(".xterm-rows")).not.toContainText(`stopped-only-${stamp}`);

    await page.locator("[data-broadcast-open]").click();
    await page.locator("[data-broadcast-start]").click();
    await b.locator("[data-pane-close]").click();
    await expect(panes(page)).toHaveCount(2);
    await expect(page.locator("[data-terminal-broadcast]")).toHaveAttribute("data-enabled", "false");
    await send(a, `echo after-close-${stamp}`);
    await expect(a.locator(".xterm-rows")).toContainText(`after-close-${stamp}`);
    await expect(c.locator(".xterm-rows")).not.toContainText(`after-close-${stamp}`);
  } finally {
    await close();
  }
});
