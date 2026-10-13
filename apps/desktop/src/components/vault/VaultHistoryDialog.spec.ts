/**
 * The archived vaults dialog: what it lists, and what each answer does.
 *
 * Three behaviours are worth a test here and none of them is about markup: a password that does not
 * open is answered in the row that asked without the live vault being touched, an import says what
 * it wrote including what it could not carry, and a switch says what it costs before it happens and
 * ends by leaving the gate on the vault that arrived.
 *
 * The transport is mocked and the store is real, which is what makes the third one checkable: the
 * store is where "which vault is live" is kept, and the dialog is the only thing that can leave it
 * saying one thing while the disk says another.
 */
import { createPinia, setActivePinia } from "pinia";
import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  vaultState: vi.fn(),
  vaultArchives: vi.fn(),
  vaultArchiveVerify: vi.fn(),
  vaultArchiveActivate: vi.fn(),
  vaultArchiveImport: vi.fn(),
  vaultRevealFolder: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
}));

import VaultHistoryDialog from "@/components/vault/VaultHistoryDialog.vue";
import { i18n } from "@/i18n";
import {
  groupsList,
  sessionsList,
  vaultArchiveActivate,
  vaultArchiveImport,
  vaultArchiveVerify,
  vaultArchives,
  vaultRevealFolder,
  vaultState,
} from "@/lib/backend/tauri";
import { useVaultStore } from "@/stores/vault";

const vaultStateMock = vi.mocked(vaultState);
const archivesMock = vi.mocked(vaultArchives);
const verifyMock = vi.mocked(vaultArchiveVerify);
const activateMock = vi.mocked(vaultArchiveActivate);
const importMock = vi.mocked(vaultArchiveImport);
const revealMock = vi.mocked(vaultRevealFolder);
const sessionsListMock = vi.mocked(sessionsList);
const groupsListMock = vi.mocked(groupsList);

const ARCHIVES = [
  { name: "vault.json.old", modifiedAt: 1_700_000_000, sizeBytes: 2048 },
  { name: "vault.json.2.old", modifiedAt: 1_600_000_000, sizeBytes: 4096 },
];

const UNLOCK_FAILED = {
  version: 1,
  code: "VAULT-UNLOCK-FAILED",
  messageKey: "errors.vault.unlockFailed",
  messageParams: {},
  origin: { subsystem: "vault" },
  outcome: "rejected",
};

let app: ReturnType<typeof createApp> | undefined;
let activated: string[] = [];
/** What the dialog asked its parent to do, which is how a dialog closes: `open` belongs to the caller. */
let asked: boolean[] = [];

async function mount(gate: "locked" | "unlocked"): Promise<void> {
  vaultStateMock.mockResolvedValue(gate);

  const pinia = createPinia();
  setActivePinia(pinia);
  await useVaultStore().initialize();

  app = createApp(VaultHistoryDialog, {
    open: true,
    onActivated: (name: string) => activated.push(name),
    "onUpdate:open": (value: boolean) => asked.push(value),
  });
  app.use(pinia);
  app.use(i18n);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  await settle();
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
}

/** The dialog is teleported to the body, so it is not inside the element it was mounted on. */
function dialog(): HTMLElement {
  const element = document.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]');
  if (!element) {
    throw new Error("the dialog is not on screen");
  }
  return element;
}

function row(name: string): HTMLElement {
  const item = [...dialog().querySelectorAll("li")].find((element) => element.textContent?.includes(name));
  if (!item) {
    throw new Error(`no row for ${name}`);
  }
  return item;
}

function buttonIn(scope: HTMLElement, label: string): HTMLButtonElement {
  const button = [...scope.querySelectorAll("button")].find((element) => element.textContent?.trim() === label);
  if (!button) {
    throw new Error(`no \`${label}\` button in ${scope.textContent?.slice(0, 60)}`);
  }
  return button;
}

/** A control whose name is on it rather than in it: an icon button, which has no text to match. */
function buttonNamed(scope: HTMLElement, label: string): HTMLButtonElement {
  const button = [...scope.querySelectorAll("button")].find((element) => element.getAttribute("aria-label") === label);
  if (!button) {
    throw new Error(`no control named \`${label}\` in ${scope.textContent?.slice(0, 60)}`);
  }
  return button;
}

function typePassword(scope: HTMLElement, value: string): void {
  const input = scope.querySelector<HTMLInputElement>("input[type='password']");
  if (!input) {
    throw new Error("the row has no password field");
  }
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

function submitPassword(scope: HTMLElement): void {
  scope.querySelector("form")?.dispatchEvent(new Event("submit", { cancelable: true }));
}

/**
 * One row with its password panel open and the password checked, which is where three of these
 * cases start: what a person sees after "Try a password" and one successful check.
 */
async function mountTrying(gate: "locked" | "unlocked" = "locked"): Promise<HTMLElement> {
  verifyMock.mockResolvedValue(undefined);
  await mount(gate);

  const first = row("vault.json.old");
  buttonIn(first, "Try a password").click();
  await settle();
  typePassword(first, "the password");
  submitPassword(first);
  await settle();
  return first;
}

describe("the archived vaults dialog", () => {
  beforeEach(() => {
    activated = [];
    asked = [];
    vaultStateMock.mockReset().mockResolvedValue("locked");
    archivesMock.mockReset().mockResolvedValue(ARCHIVES);
    verifyMock.mockReset();
    activateMock.mockReset();
    importMock.mockReset();
    revealMock.mockReset();
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    app?.unmount();
    app = undefined;
    document.body.innerHTML = "";
  });

  it("lists the archives it found, with when they were written and how big they are", async () => {
    await mount("locked");

    expect(dialog().textContent).toContain("vault.json.old");
    expect(dialog().textContent).toContain("vault.json.2.old");
    expect(dialog().textContent).toContain("2.0 KB");
    expect(dialog().textContent).toContain("4.1 KB");
  });

  it("says so when there is nothing to switch to", async () => {
    archivesMock.mockResolvedValue([]);
    await mount("locked");

    expect(dialog().textContent).toContain("No archived vaults on this device yet");
  });

  it("answers a wrong password in the row that asked, and switches nothing", async () => {
    verifyMock.mockRejectedValue(UNLOCK_FAILED);
    await mount("locked");

    const first = row("vault.json.old");
    buttonIn(first, "Try a password").click();
    await settle();

    typePassword(first, "not the password");
    submitPassword(first);
    await settle();

    expect(first.textContent).toContain("VAULT-UNLOCK-FAILED");
    expect(activateMock, "a try is not a switch").not.toHaveBeenCalled();
    expect(importMock, "and not an import").not.toHaveBeenCalled();
    expect(useVaultStore().state, "and the live vault is where it was").toBe("locked");
  });

  it("offers the import once the password opened, and says what it wrote", async () => {
    vaultStateMock.mockResolvedValue("unlocked");
    verifyMock.mockResolvedValue(undefined);
    importMock.mockResolvedValue({ sessions: 3, groups: 1, secrets: 2, missingKeys: 1, droppedHops: 0 });
    await mount("unlocked");

    const first = row("vault.json.old");
    const readsBefore = sessionsListMock.mock.calls.length;
    buttonIn(first, "Try a password").click();
    await settle();
    typePassword(first, "the password");
    submitPassword(first);
    await settle();

    expect(first.textContent).toContain("That password opens this vault");
    expect(sessionsListMock.mock.calls.length, "looking at an archive changes no library").toBe(readsBefore);

    buttonIn(first, "Import its sessions").click();
    await settle();

    expect(first.textContent).toContain("3 sessions, 1 groups and 2 credentials were added");
    expect(first.textContent, "what could not be carried is said rather than hidden").toContain(
      "1 of them name a key this device does not hold",
    );
    expect(sessionsListMock.mock.calls.length, "the library is reloaded so the new sessions are on screen").toBe(
      readsBefore + 1,
    );
    // An import adds rather than replaces, so the button does not stay there inviting a second copy.
    expect(
      [...first.querySelectorAll("button")].some((element) => element.textContent?.trim() === "Import its sessions"),
    ).toBe(false);
  });

  it("keeps the import unusable while the vault is shut, and says why on the control itself", async () => {
    const first = await mountTrying();

    const control = buttonIn(first, "Import its sessions");
    // `aria-disabled` rather than `disabled`, and the reason is the tooltip: a control with the
    // `disabled` attribute is skipped by hit testing, so nothing that hovers it ever fires and the
    // sentence would be a tooltip that cannot open. What has to hold instead is that the control is
    // *unusable* — which is what the click below checks.
    expect(control.getAttribute("aria-disabled"), "the control says it is unavailable").toBe("true");

    control.click();
    await settle();
    expect(importMock, "and pressing it imports nothing").not.toHaveBeenCalled();

    // The sentence itself is checked where it appears: on hover. A real check rather than a
    // structural one, because a tooltip nothing can open is a silent nothing.
    control.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse" }));
    await new Promise((resolve) => setTimeout(resolve, 400));

    expect(document.body.textContent, "and the reason is said out loud").toContain(
      "Unlock the master vault before merging an import into it.",
    );
  });

  it("takes the check back when the password is edited after it", async () => {
    const first = await mountTrying("unlocked");
    expect(first.textContent).toContain("That password opens this vault");
    expect(buttonIn(first, "Import its sessions")).toBeDefined();

    typePassword(first, "the password, corrected");
    await settle();

    // The check was a fact about the old value and the import would have carried the new one across
    // without anybody having opened it, so both go: what is left is a field and a button to check it.
    expect(first.textContent, "the sentence about the old value is gone").not.toContain(
      "That password opens this vault",
    );
    expect(
      [...first.querySelectorAll("button")].some((element) => element.textContent?.trim() === "Import its sessions"),
      "and nothing offers to import on the strength of it",
    ).toBe(false);
  });

  it("shows the typed password as text when asked, and keeps the eye from being the Enter key", async () => {
    const first = await mountTrying();

    const field = first.querySelector<HTMLInputElement>("input[type='password']");
    if (!field) {
      throw new Error("the try panel has no password field");
    }
    const eye = buttonNamed(first, "Show the password");
    // A button inside a form is a submit button unless it says otherwise, and the one button under
    // this field is the one Enter presses — so the eye has to opt out of that by name.
    expect(eye.type, "a button inside the form defaults to submit, which is the bug this checks").toBe("button");

    eye.click();
    await settle();

    expect(first.querySelector<HTMLInputElement>("input[type='text']")?.value).toBe("the password");
    buttonNamed(first, "Hide the password").click();
    await settle();
    expect(first.querySelector<HTMLInputElement>("input[type='password']")).not.toBeNull();
  });

  it("closes the password panel when the switch question is opened, and the other way round", async () => {
    const first = await mountTrying();

    // Both open at once is a card taller than the list it belongs to, which is the whole reason the
    // two are exclusive: one is "can this open?" and the other is "should this take the live vault's
    // place?", and nobody reads either one through a scrollbar.
    buttonIn(first, "Make it the live vault").click();
    await settle();

    expect(first.textContent).toContain("Switch to this vault?");
    expect(first.querySelector("input[type='password']"), "the password panel put itself away").toBeNull();

    buttonIn(first, "Try a password").click();
    await settle();

    expect(first.querySelector("input[type='password']")).not.toBeNull();
    expect(first.textContent, "and the question put itself away").not.toContain("Switch to this vault?");
  });

  it("spells out what a switch costs, and leaves the gate on the vault that arrived", async () => {
    activateMock.mockResolvedValue("vault.json.old");
    await mount("locked");

    const second = row("vault.json.2.old");
    buttonIn(second, "Make it the live vault").click();
    await settle();

    expect(second.textContent).toContain("Switch to this vault?");
    expect(second.textContent, "and says both halves of what it costs").toContain("no data is lost");
    expect(second.textContent).toContain("the password that archive was written with");
    expect(activateMock, "the consequence is read before anything happens").not.toHaveBeenCalled();

    // The state the store asks for afterwards: the file in place is the one that arrived, and nobody
    // has typed its password yet.
    vaultStateMock.mockResolvedValue("locked");
    buttonIn(second, "Confirm the switch").click();
    await settle();

    expect(activateMock).toHaveBeenCalledWith("vault.json.2.old");
    expect(activated, "the screen behind is told which library is live").toEqual(["vault.json.2.old"]);
    expect(useVaultStore().state).toBe("locked");
    // Closing is the caller's decision — `open` belongs to whoever drew the dialog — so what the
    // dialog owes is the request, and this is it.
    expect(asked, "and the dialog asks to be closed").toContain(false);
  });

  it("opens the folder they live in, and draws a refusal at the top of the list", async () => {
    await mount("locked");

    revealMock.mockResolvedValue(undefined);
    buttonIn(dialog(), "Open the folder").click();
    await settle();
    expect(revealMock).toHaveBeenCalled();

    revealMock.mockRejectedValue({
      version: 1,
      code: "FS-OPEN-FAILED",
      messageKey: "errors.fs.openFailed",
      messageParams: {},
      origin: { subsystem: "fs" },
      outcome: "unknown",
    });
    buttonIn(dialog(), "Open the folder").click();
    await settle();

    expect(dialog().textContent).toContain("FS-OPEN-FAILED");
  });
});
