import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";
const LAB_HOST = process.env.SEVENSHELL_TEST_HOST;
const LAB_PORT = process.env.SEVENSHELL_TEST_PORT ?? "22";
const LAB_USER = process.env.SEVENSHELL_TEST_USER;
const LAB_PASSWORD = process.env.SEVENSHELL_TEST_PASSWORD;
const SSH_CONTAINER = "sevenshell-lab-sshd";
const K8S_CONTAINER = "sevenshell-lab-k3s";
const HERE = path.dirname(fileURLToPath(import.meta.url));

interface TestWindow {
  __TAURI_INTERNALS__: {
    invoke: (command: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
  };
  __editorClipboardUploads?: number;
  __editorReleaseSave?: () => void;
}

async function invoke<T>(page: Page, command: string, args?: Record<string, unknown>): Promise<T> {
  return page.evaluate(
    ({ command, args }) => (window as unknown as TestWindow).__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  ) as Promise<T>;
}

async function openApp(): Promise<{ page: Page; close: () => Promise<void> }> {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  const gate = page.getByRole("heading", { name: "Create the vault" });
  const library = page.locator("[data-session-list]");
  await expect(gate.or(library)).toBeVisible({ timeout: 60_000 });
  if (await gate.isVisible()) {
    await page.getByRole("textbox", { name: "Password", exact: true }).fill("e2e-text-editor-password");
    await page.getByRole("textbox", { name: "Confirm password" }).fill("e2e-text-editor-password");
    await page.getByRole("button", { name: "Create vault" }).click();
    await expect(library).toBeVisible({ timeout: 60_000 });
  }
  const baseline = await invoke<Record<string, unknown>>(page, "settings_get");
  await invoke(page, "settings_set", {
    settings: {
      ...baseline,
      locale: "en",
      sessionLayout: "split",
      terminalPanelPlacement: "bottom",
      editorIndentWidth: 4,
      editorIndentStyle: "spaces",
      editorAutoIndent: false,
      keybindings: {},
    },
  });
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(library).toBeVisible({ timeout: 60_000 });

  return {
    page,
    close: async () => {
      try {
        await invoke(page, "settings_set", { settings: baseline });
        const discard = page.getByRole("button", { name: "Discard changes", exact: true });
        if (await discard.isVisible()) {
          await discard.click();
        }
        for (let guard = 0; guard < 4 && (await page.getByRole("dialog").count()) > 0; guard++) {
          const cancel = page.getByRole("dialog").last().getByRole("button", { name: "Cancel", exact: true });
          if (await cancel.isVisible()) {
            await cancel.click();
          } else {
            await page.keyboard.press("Escape");
          }
          if (await discard.isVisible()) {
            await discard.click();
          }
        }
        const tabs = page.locator("[data-workspace-tabs] [data-tab]");
        for (let guard = 0; guard < 12 && (await tabs.count()) > 0; guard++) {
          const remaining = await tabs.count();
          await tabs.last().locator("button").last().click({ timeout: 5_000 });
          await expect(tabs).toHaveCount(remaining - 1, { timeout: 5_000 });
        }
        await page.reload({ waitUntil: "domcontentloaded" });
        await expect(library).toBeVisible({ timeout: 60_000 });
      } finally {
        await browser.close();
      }
    },
  };
}

function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

async function choose(page: Page, field: Locator, option: string, exact = true): Promise<void> {
  await field.click();
  await page.getByRole("option", { name: option, exact }).first().click();
}

/** Small fixtures fit in the viewport; read the rendered lines without normalizing their spaces. */
async function buffer(field: Locator): Promise<string> {
  return field.evaluate((element) =>
    [...element.querySelectorAll(":scope > .cm-line")].map((line) => line.textContent ?? "").join("\n"),
  );
}

async function fillEditor(page: Page, field: Locator, text: string): Promise<void> {
  await field.focus();
  await field.press("Control+A");
  await page.keyboard.insertText(text);
  await expect.poll(() => buffer(field)).toBe(text);
}

test("the text editor keeps Tab, undo and native paste in the SFTP buffer and saves from the keyboard", async () => {
  test.skip(!LAB_HOST || !LAB_USER || !LAB_PASSWORD, "the SSH lab is not configured");
  test.skip(LAB_HOST !== "127.0.0.1", "this fixture belongs to the disposable local SSH container");
  const stamp = Date.now();
  const name = `editor-${stamp}`;
  const fileName = `sshell-editor-${stamp}.conf`;
  const filePath = `/tmp/${fileName}`;
  const original = "alpha\nbeta\n";
  execFileSync("docker", [
    "exec",
    "-u",
    "labuser",
    SSH_CONTAINER,
    "sh",
    "-c",
    `printf 'alpha\nbeta\n' > '${filePath}'`,
  ]);
  const { page, close } = await openApp();
  const previousClipboard = await invoke<string>(page, "clipboard_text");
  try {
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const session = page.getByRole("dialog");
    await session.getByPlaceholder("Production web server").fill(name);
    await session.getByPlaceholder("example.com").fill(LAB_HOST as string);
    await session.locator("#session-port").fill(LAB_PORT);
    await session.getByPlaceholder("root").fill(LAB_USER as string);
    await session.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await session.getByRole("button", { name: "Save", exact: true }).click();
    await expect(session).toHaveCount(0);
    await sidebar(page).getByTitle(`Open a terminal for ${name}`).click();
    await page.mouse.move(5, 5);
    const question = page.getByRole("button", { name: "Trust this key and connect" });
    const crumbs = page.getByRole("navigation", { name: "Path" }).filter({ visible: true });
    await expect(question.or(crumbs).first()).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }
    await expect(crumbs).toBeVisible({ timeout: 30_000 });
    await page.getByRole("listbox", { name: "Files", exact: true }).filter({ visible: true }).focus();
    await page.keyboard.press("Control+L");
    await page.getByPlaceholder("Path").filter({ visible: true }).fill("/tmp");
    await page.getByPlaceholder("Path").filter({ visible: true }).press("Enter");
    const row = page.getByRole("option").filter({ hasText: fileName });
    await expect(row).toBeVisible({ timeout: 30_000 });
    await row.locator("[data-file-name]").dblclick();
    const dialog = page.getByRole("dialog");
    const field = dialog.locator("[data-text-editor] .cm-content");
    await expect(field).toHaveAttribute("aria-readonly", "false");
    await expect.poll(() => buffer(field)).toBe(original);
    await field.focus();
    await field.press("Control+Home");
    await field.press("Tab");
    await expect(field).toBeFocused();
    await expect.poll(() => buffer(field)).toBe("    alpha\nbeta\n");
    await field.press("Control+Z");
    await expect.poll(() => buffer(field)).toBe(original);
    await field.press("Control+Shift+Z");
    await expect.poll(() => buffer(field)).toBe("    alpha\nbeta\n");
    await field.press("Shift+Tab");
    await expect.poll(() => buffer(field)).toBe(original);
    await field.press("Control+A");
    await field.press("Tab");
    await expect.poll(() => buffer(field)).toBe("    alpha\n    beta\n");
    await field.press("Shift+Tab");
    await expect.poll(() => buffer(field)).toBe(original);

    await field.press("Control+Shift+M");
    await field.press("Tab");
    await expect(dialog.getByRole("button", { name: "Tab: move focus", exact: true })).toBeFocused();
    await field.focus();
    await field.press("Control+Shift+M");
    await field.press("Control+Home");
    await field.press("Tab");
    await field.press("Control+End");
    await page.evaluate(async () => {
      const probe = window as unknown as TestWindow;
      const moduleUrl = "/src/lib/backend/api.ts";
      const { backend } = await import(moduleUrl);
      const originalWrite = backend.files.writeText;
      const originalClipboard = backend.files.clipboardFiles;
      probe.__editorClipboardUploads = 0;
      let holdNextSave = true;
      backend.files.clipboardFiles = () => {
        probe.__editorClipboardUploads = (probe.__editorClipboardUploads ?? 0) + 1;
        return originalClipboard();
      };
      backend.files.writeText = (...args: [string, string, string]) => {
        if (holdNextSave) {
          holdNextSave = false;
          return new Promise((resolve, reject) => {
            probe.__editorReleaseSave = () => {
              delete probe.__editorReleaseSave;
              originalWrite(...args).then(resolve, reject);
            };
          });
        }
        return originalWrite(...args);
      };
      if (backend.files.writeText === originalWrite) {
        throw new Error("the delayed-save fixture was not installed");
      }
      await navigator.clipboard.writeText("paste-from-window");
    });
    await field.press("Control+V");
    const saved = "    alpha\nbeta\npaste-from-window";
    await expect.poll(() => buffer(field)).toBe(saved);
    await field.press("Control+S");
    await expect(field).toHaveAttribute("aria-busy", "true");
    await expect(field).toHaveAttribute("aria-readonly", "true");
    await field.press("Control+V");
    expect(await buffer(field)).toBe(saved);
    expect(await page.evaluate(() => (window as unknown as TestWindow).__editorClipboardUploads)).toBe(0);
    await page.evaluate(() => (window as unknown as TestWindow).__editorReleaseSave?.());
    await expect(field).toHaveAttribute("aria-busy", "false");
    await expect
      .poll(() => execFileSync("docker", ["exec", SSH_CONTAINER, "cat", filePath], { encoding: "utf8" }))
      .toBe(saved);
    await expect(field).toBeFocused();
    expect(await page.evaluate(() => (window as unknown as TestWindow).__editorClipboardUploads)).toBe(0);
    await page.screenshot({ path: path.join(HERE, "artifacts", "text-editor-sftp.png") });

    await page.keyboard.type(" unsaved");
    await field.press("Escape");
    await expect(page.getByRole("dialog").filter({ hasText: "Unsaved changes" })).toBeVisible();
    await page.getByRole("button", { name: "Keep editing", exact: true }).click();
    await expect(field).toBeFocused();
    await field.press("Control+Enter");
    await expect(dialog).toHaveCount(0);
    await expect
      .poll(() => execFileSync("docker", ["exec", SSH_CONTAINER, "cat", filePath], { encoding: "utf8" }))
      .toBe(`${saved} unsaved`);
    await row.locator("[data-file-name]").dblclick();
    await expect.poll(() => buffer(field)).toBe(`${saved} unsaved`);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  } finally {
    await page.evaluate(() => (window as unknown as TestWindow).__editorReleaseSave?.()).catch(() => {});
    await page.evaluate((value) => navigator.clipboard.writeText(value), previousClipboard).catch(() => {});
    execFileSync("docker", ["exec", SSH_CONTAINER, "rm", "-f", filePath], { stdio: "ignore" });
    await close();
  }
});

test("Kubernetes text editors use the saved indentation preferences and keep failed YAML drafts", async () => {
  const kubeconfigPath = path.join(HERE, "..", ".scratch", "k3s", "kubeconfig-original.yaml");
  test.skip(!existsSync(kubeconfigPath), "the k3s lab kubeconfig is not present");
  const kubeconfig = await readFile(kubeconfigPath, "utf8");
  const name = `editor-cm-${Date.now()}`;
  const { page, close } = await openApp();
  try {
    await page.getByRole("banner").getByRole("button", { name: "Settings", exact: true }).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Files", exact: true }).click();
    await choose(
      page,
      settings.getByRole("combobox", { name: "Text editor indentation width", exact: true }),
      "2 columns",
    );
    await choose(
      page,
      settings.getByRole("combobox", { name: "Text editor indentation characters", exact: true }),
      "Tab character",
    );
    await settings.getByRole("switch", { name: "Keep indentation on Enter", exact: true }).click();
    await settings.getByRole("button", { name: "Apply", exact: true }).click();
    await expect(settings.getByRole("button", { name: "Apply", exact: true })).toBeDisabled();
    expect(await invoke(page, "settings_get")).toMatchObject({
      editorIndentWidth: 2,
      editorIndentStyle: "tabs",
      editorAutoIndent: true,
    });
    await page.getByRole("button", { name: "Close settings", exact: true }).click();

    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const session = page.getByRole("dialog");
    await session.getByPlaceholder("Production web server").fill(name);
    await choose(page, session.locator("#session-protocol"), "Kubernetes");
    await session.getByRole("radio", { name: "Import a kubeconfig" }).click();
    await session.locator("#session-kubeconfig").fill(kubeconfig);
    await session.getByRole("button", { name: "Read contexts" }).click();
    await choose(page, session.locator("#session-kube-context"), "default", false);
    await session.getByRole("button", { name: "Save", exact: true }).click();
    await expect(session).toHaveCount(0);
    await sidebar(page).getByTitle(`Open a terminal for ${name}`).click();
    await page.mouse.move(5, 5);
    const tree = page.getByRole("navigation", { name: "Cluster resources" });
    await expect(tree).toBeVisible({ timeout: 30_000 });
    await tree.getByRole("button", { name: "Config maps", exact: true }).click();
    await page.getByLabel("Namespace", { exact: true }).fill("default");
    await page.getByRole("button", { name: "Create", exact: true }).click();
    const create = page.getByRole("dialog");
    const field = create.locator("[data-text-editor] .cm-content");
    const manifest = `apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: ${name}\n  namespace: default\ndata:\n  mode: original\n`;
    await fillEditor(page, field, manifest);
    await field.press("Control+A");
    await field.press("Tab");
    await expect
      .poll(() => buffer(field))
      .toBe(
        manifest
          .split("\n")
          .map((line) => (line ? `  ${line}` : line))
          .join("\n"),
      );
    await expect(create.getByText("YAML uses spaces", { exact: true })).toBeVisible();
    await field.press("Shift+Tab");
    await expect.poll(() => buffer(field)).toBe(manifest);
    await field.press("Control+Enter");
    await expect(create).toHaveCount(0);
    const row = page
      .getByRole("table", { name: "Config maps list", exact: true })
      .getByRole("row")
      .filter({ hasText: name });
    await expect(row).toBeVisible({ timeout: 30_000 });
    await row.click();
    await page.getByRole("tab", { name: "YAML", exact: true }).click();
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    const draft = page.locator("aside [data-text-editor] .cm-content");
    const current = await buffer(draft);
    expect(current).toContain("mode: original");
    await fillEditor(page, draft, current.replace("mode: original", "mode: updated"));
    await draft.press("Control+S");
    await expect(draft).toHaveCount(0);
    await expect
      .poll(() =>
        execFileSync(
          "docker",
          ["exec", K8S_CONTAINER, "kubectl", "get", "configmap", name, "-n", "default", "-o", "jsonpath={.data.mode}"],
          { encoding: "utf8" },
        ),
      )
      .toBe("updated");
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    await fillEditor(page, draft, "kind: [unclosed");
    await draft.press("Control+Enter");
    await expect(page.getByRole("alert").filter({ hasText: "The YAML could not be read." })).toBeVisible();
    await expect.poll(() => buffer(draft)).toBe("kind: [unclosed");
    await page.screenshot({ path: path.join(HERE, "artifacts", "text-editor-kubernetes.png") });
    await draft.press("Escape");
    await expect(draft).toHaveCount(0);
  } finally {
    execFileSync(
      "docker",
      ["exec", K8S_CONTAINER, "kubectl", "delete", "configmap", name, "-n", "default", "--ignore-not-found"],
      { stdio: "ignore" },
    );
    await close();
  }
});
