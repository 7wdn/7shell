/**
 * The sync page: what a backup holds, writing one, and bringing one back.
 *
 * The rules of the file format and of a restore are tested in Rust, where they live. This file is
 * about the surface: which question is asked before a backup is written, what a refused passphrase
 * does, and which parts of the window are read again after a restore.
 */
import { createPinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  syncAvailable: vi.fn(),
  syncBackupExport: vi.fn(),
  syncBackupInspect: vi.fn(),
  syncBackupRestore: vi.fn(),
  syncStatus: vi.fn(),
  syncPassword: vi.fn(),
  syncPassphrase: vi.fn(),
  syncTest: vi.fn(),
  syncUpload: vi.fn(),
  syncDownload: vi.fn(),
  syncApply: vi.fn(),
  snippetSyncStatus: vi.fn(),
  snippetSyncToken: vi.fn(),
  snippetSyncTest: vi.fn(),
  snippetSyncUpload: vi.fn(),
  snippetSyncDownload: vi.fn(),
  snippetSyncApply: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
  snippetsOpen: vi.fn(),
  completionWords: vi.fn(),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
}));

import SyncSettings from "@/components/settings/SyncSettings.vue";
import { i18n } from "@/i18n";
import { ShellError } from "@/lib/backend/errors";
import {
  completionWords,
  groupsList,
  sessionsList,
  settingsGet,
  settingsSet,
  snippetSyncApply,
  snippetSyncDownload,
  snippetSyncStatus,
  snippetSyncTest,
  snippetSyncToken,
  snippetSyncUpload,
  snippetsOpen,
  syncApply,
  syncAvailable,
  syncBackupExport,
  syncBackupInspect,
  syncBackupRestore,
  syncDownload,
  syncPassphrase,
  syncPassword,
  syncStatus,
  syncTest,
  syncUpload,
} from "@/lib/backend/tauri";
import type { SyncApplySummary, SyncCatalog, SyncContents } from "@/types/sync";

const availableMock = vi.mocked(syncAvailable);
const exportMock = vi.mocked(syncBackupExport);
const inspectMock = vi.mocked(syncBackupInspect);
const restoreMock = vi.mocked(syncBackupRestore);
const sessionsMock = vi.mocked(sessionsList);
const groupsMock = vi.mocked(groupsList);
const snippetsMock = vi.mocked(snippetsOpen);
const wordsMock = vi.mocked(completionWords);
const settingsMock = vi.mocked(settingsGet);
const settingsSetMock = vi.mocked(settingsSet);
const statusMock = vi.mocked(syncStatus);
const passwordMock = vi.mocked(syncPassword);
const passphraseMock = vi.mocked(syncPassphrase);
const testMock = vi.mocked(syncTest);
const uploadMock = vi.mocked(syncUpload);
const downloadMock = vi.mocked(syncDownload);
const applyMock = vi.mocked(syncApply);
const snippetStatusMock = vi.mocked(snippetSyncStatus);
const snippetTokenMock = vi.mocked(snippetSyncToken);
const snippetTestMock = vi.mocked(snippetSyncTest);
const snippetUploadMock = vi.mocked(snippetSyncUpload);
const snippetDownloadMock = vi.mocked(snippetSyncDownload);
const snippetApplyMock = vi.mocked(snippetSyncApply);

let app: App | undefined;
let host: HTMLDivElement;
/** The one token the mocked vault holds, so "keep it" and "is one kept" cannot disagree. */
let keptToken: string | null = null;

function contents(overrides: Partial<SyncContents> = {}): SyncContents {
  return {
    sessions: 2,
    groups: 1,
    snippets: 3,
    words: 1,
    history: 0,
    knownHosts: 0,
    settings: 18,
    secrets: 0,
    keys: 0,
    ...overrides,
  };
}

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(SyncSettings) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await vi.waitFor(() => {
    expect(availableMock).toHaveBeenCalled();
  });
  await nextTick();
}

/** The dialog is drawn in a portal, so it lies beside the page rather than inside it. */
function dialog(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>('[role="dialog"]');
}

async function press(label: string, where: ParentNode = host): Promise<void> {
  const button = [...where.querySelectorAll("button")].find((candidate) => candidate.textContent?.trim() === label);
  if (!button) {
    throw new Error(`no button reading ${label}`);
  }
  button.click();
  await nextTick();
}

/** The switch beside a label, in the dialog that asks what a backup holds. */
function toggle(label: string): HTMLInputElement {
  const row = [...(dialog()?.querySelectorAll("label") ?? [])].find((candidate) =>
    candidate.textContent?.includes(label),
  );
  const box = row?.querySelector<HTMLInputElement>("input[type=checkbox]");
  if (!box) {
    throw new Error(`no switch for ${label}`);
  }
  return box;
}

async function typeInto(id: string, value: string): Promise<void> {
  const element = host.querySelector<HTMLInputElement>(id.startsWith("#") ? id : `#${id}`);
  if (!element) {
    throw new Error(`the panel has no field ${id}`);
  }
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await nextTick();
}

async function type(value: string): Promise<void> {
  const element = host.querySelector<HTMLInputElement>("#sync-passphrase");
  if (!element) {
    throw new Error("the panel has no passphrase field");
  }
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await nextTick();
}

/** Opens the file panel, because the page now opens on the server's. */
async function openLocal(): Promise<void> {
  await openTab("Local backup");
}

/** Opens any of the three panels by the label on its tab. */
async function openTab(label: string): Promise<void> {
  const tab = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  candidate_or_throw(tab, `the ${label} tab`);
  tab.click();
  await nextTick();
}

function candidate_or_throw(tab: HTMLButtonElement | undefined, what: string): asserts tab is HTMLButtonElement {
  if (!tab) {
    throw new Error(`no ${what}`);
  }
}

beforeEach(() => {
  availableMock.mockReset().mockResolvedValue(contents());
  exportMock.mockReset();
  inspectMock.mockReset();
  restoreMock.mockReset();
  sessionsMock.mockReset().mockResolvedValue([]);
  groupsMock.mockReset().mockResolvedValue([]);
  snippetsMock.mockReset().mockResolvedValue([]);
  wordsMock.mockReset().mockResolvedValue([]);
  // The settings are only ever *read again* by this page, never inspected: an empty object is
  // enough to see whether the call happened, and the shape of one is another spec's business.
  settingsMock.mockReset().mockResolvedValue({} as never);
  settingsSetMock.mockReset().mockResolvedValue(undefined);
  statusMock.mockReset().mockResolvedValue({ hasPassword: false, hasPassphrase: false });
  passwordMock.mockReset().mockImplementation(async (password: string | null) => ({
    hasPassword: password !== null,
    hasPassphrase: false,
  }));
  passphraseMock.mockReset().mockImplementation(async (passphrase: string | null) => ({
    hasPassword: false,
    hasPassphrase: passphrase !== null,
  }));
  testMock.mockReset().mockResolvedValue(undefined);
  uploadMock.mockReset().mockResolvedValue({ contents: contents(), sealed: false, remotePath: "sync/backup.json" });
  downloadMock.mockReset().mockResolvedValue({ kind: "nothingYet" });
  applyMock.mockReset().mockResolvedValue({
    contents: contents(),
    replaced: 0,
    copied: 0,
    missingKeys: 0,
    droppedHops: 0,
  });
  // A fresh account: the shell has answered with the name of the service's one account and says no
  // token is kept, which is what the snippet panel's fields are drawn from. What is kept is held
  // here so the two answers stay consistent — a real vault holds one token, and a mock where
  // "keep it" and "is it kept" disagree is a mock that fails a passing application.
  keptToken = null;
  snippetStatusMock.mockReset().mockImplementation(async () => ({
    account: "github",
    hasToken: keptToken !== null,
  }));
  snippetTokenMock.mockReset().mockImplementation(async (token: string | null) => {
    keptToken = token;
    return { account: "github", hasToken: token !== null };
  });
  snippetTestMock.mockReset().mockResolvedValue(undefined);
  snippetUploadMock.mockReset().mockResolvedValue({
    contents: contents(),
    sealed: true,
    provider: "github",
    snippetId: "abc123",
  });
  snippetDownloadMock.mockReset().mockResolvedValue({
    kind: "read",
    catalog: { contents: contents(), sealed: true, exportedAt: 1, appVersion: "0.1.0" },
  });
  snippetApplyMock.mockReset().mockResolvedValue({
    contents: contents(),
    replaced: 0,
    copied: 0,
    missingKeys: 0,
    droppedHops: 0,
  });
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="dialog"]')) {
    node.remove();
  }
});

describe("the sync page", () => {
  it("offers the three places a backup can live, all of them open", async () => {
    await mount();

    expect(host.textContent).toContain("Local backup");
    const tabs = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(
      tabs.map((tab) => tab.disabled),
      "none of the three is waiting to be built",
    ).toEqual([false, false, false]);

    await openTab("Snippet sync");
    expect(host.textContent).toContain("GitHub Gist");
    expect(host.querySelector("#snippet-token"), "and its form is drawn").not.toBeNull();
  });

  it("says there is nothing to test with until a token is entered", async () => {
    await mount();
    await openTab("Snippet sync");

    await press("Test");

    // The shell would answer the same way, but a round trip to be told "no token" is a round trip
    // spent saying what the panel already knows.
    await vi.waitFor(() => expect(snippetStatusMock).toHaveBeenCalled());
    expect(host.textContent).toContain("Enter an access token first, or keep one.");
    expect(snippetTestMock).not.toHaveBeenCalled();
  });

  it("keeps the token in the vault before it tests, so the test has something to dial with", async () => {
    await mount();
    await openTab("Snippet sync");

    await typeInto("snippet-token", "ghp_secret");
    const box = [...host.querySelectorAll<HTMLInputElement>("input[type=checkbox]")].find((candidate) =>
      candidate.closest("label")?.textContent?.includes("Keep the token in the vault"),
    );
    if (!box) {
      throw new Error("no box for keeping the token");
    }
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    await nextTick();

    await press("Test");

    await vi.waitFor(() => expect(snippetTestMock).toHaveBeenCalled());
    expect(snippetTokenMock.mock.calls[0]?.[0], "the token goes into the vault rather than to the request").toBe(
      "ghp_secret",
    );
    expect(host.textContent).toContain("The server answered.");
  });

  it("uploads into the snippet and writes the id down under the name the shell gave the account", async () => {
    await mount();
    await openTab("Snippet sync");
    await typeInto("snippet-token", "ghp_secret");

    await press("Upload");
    // The dialog's own action keeps the words the other two panels' dialogs use: the question is
    // the same one, and the answer is "yes, back this up".
    await press("Export backup", dialog() as HTMLElement);
    await vi.waitFor(() => expect(snippetUploadMock).toHaveBeenCalled());
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalled());

    const written = settingsSetMock.mock.calls.at(-1)?.[0] as { syncSnippetIds?: Record<string, string> };
    expect(written.syncSnippetIds, "the id is filed under the account, which is what the next upload patches").toEqual({
      github: "abc123",
    });
    expect(host.textContent).toContain("Uploaded");
  });

  it("offers no upload until there is a token, because a snippet is always sealed and always needs one", async () => {
    await mount();
    await openTab("Snippet sync");

    const upload = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
      (candidate) => candidate.textContent?.trim() === "Upload",
    );
    expect(upload?.disabled, "pressing it would only be refused by the shell").toBe(true);
  });

  it("writes the service and its address down before it asks the shell about the new account", async () => {
    await mount();
    await openTab("Snippet sync");
    snippetStatusMock.mockResolvedValue({ account: "gitlab:https://gitlab.com", hasToken: false });

    // The dropdown is the interface's own control, and choosing in it is a pointer released over a
    // row of a list that lives on the body.
    host
      .querySelector<HTMLElement>('[role="combobox"]')
      ?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
    host.querySelector<HTMLElement>('[role="combobox"]')?.click();
    await nextTick();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const gitlab = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((option) =>
      option.textContent?.includes("GitLab"),
    );
    gitlab?.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "mouse" }));

    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalled());
    const written = settingsSetMock.mock.calls.at(-1)?.[0] as {
      syncSnippetProvider?: string;
      syncSnippetInstance?: string;
    };
    expect(written.syncSnippetProvider, "the service is a setting, written before it is used").toBe("gitlab");
    expect(written.syncSnippetInstance, "and the address an empty field means").toBe("https://gitlab.com");

    // The account changed with the service, so the shell is the one asked what it holds now — and it
    // is asked after the choice is written, not before.
    const writtenAt = settingsSetMock.mock.invocationCallOrder.at(-1) ?? 0;
    const askedAt = snippetStatusMock.mock.invocationCallOrder.at(-1) ?? 0;
    expect(askedAt, "the shell is asked after the setting is written").toBeGreaterThan(writtenAt);
  });

  it("writes a backup with the switches that were ticked, and says what it wrote", async () => {
    exportMock.mockResolvedValue({
      contents: contents(),
      sealed: false,
      fileName: "7shell-backup.json",
    });
    await mount();
    await openLocal();

    await press("Export backup");
    expect(toggle("Command history").checked).toBe(false);

    toggle("Command history").checked = true;
    toggle("Command history").dispatchEvent(new Event("change"));
    await nextTick();
    await press("Export backup", dialog() as HTMLElement);

    await vi.waitFor(() => expect(exportMock).toHaveBeenCalled());
    expect(exportMock.mock.calls[0]?.[2]).toEqual({
      credentials: false,
      history: true,
      knownHosts: false,
      settings: true,
    });
    expect(exportMock.mock.calls[0]?.[3]).toBeNull();
    await vi.waitFor(() => expect(host.textContent).toContain("without any passwords"));
  });

  it("writes the address down before it dials it", async () => {
    // The shell reads the server out of the settings file, so a test pressed with an unsaved
    // address on screen would dial the saved one — the mistake somebody makes once and then
    // distrusts the button for.
    await mount();
    await typeInto("#sync-endpoint", "https://dav.example.com/dav/");

    await press("Test");

    await vi.waitFor(() => expect(testMock).toHaveBeenCalled());
    expect(settingsSetMock, "the address went into the file first").toHaveBeenCalled();
    expect(settingsSetMock.mock.invocationCallOrder[0]!).toBeLessThan(testMock.mock.invocationCallOrder[0]!);
    await vi.waitFor(() => expect(host.textContent).toContain("The server answered"));
  });

  it("keeps the password when the box asks for it, and forgets it when the box is cleared", async () => {
    await mount();
    await typeInto("#sync-endpoint", "https://dav.example.com/dav/");
    await typeInto("#sync-password", "hunter2");

    const box = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    if (!box) {
      throw new Error("the server panel has no box for keeping the password");
    }
    // Off by default, because a password nobody asked to keep is one this application has no
    // business holding.
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    await nextTick();

    await press("Test");
    await vi.waitFor(() => expect(passwordMock).toHaveBeenCalled());
    expect(passwordMock.mock.calls[0]?.[0]).toBe("hunter2");
    expect(host.querySelector<HTMLInputElement>("#sync-password")?.value, "and the field is cleared").toBe("");

    // Unticking it is forgetting rather than leaving it alone: there is one control for this, and
    // it has to mean something.
    const again = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    if (!again) {
      throw new Error("the box went missing");
    }
    again.checked = false;
    again.dispatchEvent(new Event("change"));
    await nextTick();
    await press("Test");

    await vi.waitFor(() => expect(passwordMock).toHaveBeenCalledTimes(2));
    expect(passwordMock.mock.calls[1]?.[0]).toBeNull();
  });

  it("says a server with nothing on it is nothing to worry about", async () => {
    downloadMock.mockResolvedValue({ kind: "nothingYet" });
    await mount();
    await typeInto("#sync-endpoint", "https://dav.example.com/dav/");

    await press("Download");

    await vi.waitFor(() => expect(host.textContent).toContain("has no backup there yet"));
    expect(dialog(), "and nothing to decide about a file that is not there").toBeNull();
  });

  it("will not offer to seal the passwords when there is nothing to seal them under", async () => {
    await mount();
    await openLocal();

    await press("Export backup");

    expect(toggle("Saved passwords and imported keys").disabled).toBe(true);
    expect(dialog()?.textContent).toContain("Needs the passphrase");
  });

  it("sends the passphrase that is on the panel, and expects the file to be sealed", async () => {
    exportMock.mockResolvedValue({
      contents: contents({ secrets: 2, keys: 1 }),
      sealed: true,
      fileName: "7shell-backup.json",
    });
    await mount();
    await openLocal();

    await type("correct horse");
    await press("Export backup");
    const box = toggle("Saved passwords and imported keys");
    expect(box.disabled).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    await nextTick();
    await press("Export backup", dialog() as HTMLElement);

    await vi.waitFor(() => expect(exportMock).toHaveBeenCalled());
    expect(exportMock.mock.calls[0]?.[3]).toBe("correct horse");
  });

  it("asks what is in the file before it writes anything over the library", async () => {
    const catalog: SyncCatalog = { contents: contents(), sealed: false, exportedAt: 1, appVersion: "0.1.0" };
    inspectMock.mockResolvedValue({ kind: "read", catalog, fileName: "library.json" });
    restoreMock.mockResolvedValue({
      contents: contents(),
      replaced: 0,
      copied: 2,
      missingKeys: 0,
      droppedHops: 0,
    } satisfies SyncApplySummary);
    await mount();
    await openLocal();

    await press("Import backup");

    await vi.waitFor(() => expect(inspectMock).toHaveBeenCalled());
    expect(inspectMock.mock.calls[0]?.[1]).toBeNull();
    await vi.waitFor(() => expect(dialog()?.textContent).toContain("library.json"));

    // The replacement switch starts off every time: a remembered "yes" is how a library is written
    // over a week later by somebody who meant to add to it.
    const replace = [...(dialog()?.querySelectorAll("label") ?? [])]
      .find((label) => label.textContent?.includes("Replace items with the same id"))
      ?.querySelector<HTMLInputElement>("input[type=checkbox]");
    expect(replace?.checked).toBe(false);

    await press("Import backup", dialog() as HTMLElement);

    await vi.waitFor(() => expect(restoreMock).toHaveBeenCalled());
    expect(restoreMock.mock.calls[0]?.[0]).toBe(false);
    await vi.waitFor(() => expect(host.textContent).toContain("arrived as copies"));
  });

  it("reads the library again after a restore, and the settings only when they travelled", async () => {
    const catalog: SyncCatalog = {
      contents: contents({ settings: 0 }),
      sealed: false,
      exportedAt: 1,
      appVersion: "0.1.0",
    };
    inspectMock.mockResolvedValue({ kind: "read", catalog, fileName: "library.json" });
    restoreMock.mockResolvedValue({
      contents: contents({ settings: 0 }),
      replaced: 0,
      copied: 0,
      missingKeys: 0,
      droppedHops: 0,
    } satisfies SyncApplySummary);
    await mount();
    await openLocal();
    sessionsMock.mockClear();
    settingsMock.mockClear();

    await press("Import backup");
    await vi.waitFor(() => expect(dialog()).not.toBeNull());
    await press("Import backup", dialog() as HTMLElement);

    await vi.waitFor(() => expect(sessionsMock).toHaveBeenCalled());
    expect(settingsMock, "the file carried no settings, so none were read again").not.toHaveBeenCalled();
  });

  it("says a refused passphrase beside the field that was typed in, and keeps the file", async () => {
    inspectMock.mockRejectedValue(
      new ShellError({
        version: 1,
        code: "SYNC-PASSPHRASE",
        messageKey: "errors.sync.passphrase",
        messageParams: {},
        origin: { subsystem: "store" },
        outcome: "notSent",
      }),
    );
    await mount();
    await openLocal();

    await type("wrong horse");
    await press("Import backup");

    await vi.waitFor(() => expect(host.textContent).toContain("did not open this backup"));
    expect(dialog(), "and no question about what is inside a file that did not open").toBeNull();
    expect(availableMock).toHaveBeenCalledTimes(1);
  });
});
