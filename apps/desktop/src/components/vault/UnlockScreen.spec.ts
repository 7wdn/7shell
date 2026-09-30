/**
 * What the gate screen does after a button is pressed.
 *
 * The store is the real one and only the transport is mocked, because the behaviour under test is
 * the pairing between a failure and the control that caused it: the store records which operation
 * failed, and this screen is the only thing that can get the pairing wrong.
 */
import { createPinia, setActivePinia } from "pinia";
import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  vaultState: vi.fn(),
  vaultCreate: vi.fn(),
  vaultUnlock: vi.fn(),
  vaultLock: vi.fn(),
  vaultSetAside: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
}));

import UnlockScreen from "@/components/vault/UnlockScreen.vue";
import { i18n } from "@/i18n";
import { groupsList, sessionsList, vaultSetAside, vaultState, vaultUnlock } from "@/lib/backend/tauri";
import { useVaultStore } from "@/stores/vault";

const vaultStateMock = vi.mocked(vaultState);
const vaultUnlockMock = vi.mocked(vaultUnlock);
const vaultSetAsideMock = vi.mocked(vaultSetAside);
const sessionsListMock = vi.mocked(sessionsList);
const groupsListMock = vi.mocked(groupsList);

const WRONG_PASSWORD = {
  version: 1,
  code: "VAULT-UNLOCK-FAILED",
  messageKey: "errors.vault.unlockFailed",
  messageParams: {},
  origin: { subsystem: "vault" },
  outcome: "rejected",
};

const SET_ASIDE_FAILED = {
  version: 1,
  code: "STORE-IO",
  messageKey: "errors.store.io",
  messageParams: {},
  origin: { subsystem: "store" },
  outcome: "unknown",
};

const mounted: Array<() => void> = [];

/** Mounts the screen the way the application does, and waits for the gate to settle. */
async function mountLocked(): Promise<HTMLElement> {
  vaultStateMock.mockResolvedValue("locked");

  const pinia = createPinia();
  setActivePinia(pinia);
  const app = createApp(UnlockScreen);
  app.use(pinia);
  app.use(i18n);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  mounted.push(() => {
    app.unmount();
    root.remove();
  });

  await settle();
  await useVaultStore().initialize();
  await settle();
  return root;
}

/** Lets the store's awaited promises and the render they schedule run to completion. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
}

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

function submit(root: HTMLElement): void {
  root.querySelector("form")?.dispatchEvent(new Event("submit", { cancelable: true }));
}

const passwordField = (root: HTMLElement): HTMLInputElement => {
  const input = root.querySelector<HTMLInputElement>("#vault-password");
  if (!input) {
    throw new Error("the unlock form has no password field");
  }
  return input;
};

/** The fallback block that offers to move the file aside, found by the sentence that introduces it. */
const fallbackBlock = (root: HTMLElement): Element => {
  const explanation = [...root.querySelectorAll("p")].find((element) =>
    element.textContent?.includes("Cannot open it?"),
  );
  const block = explanation?.parentElement;
  if (!block) {
    throw new Error("the unlock screen has no set-aside block");
  }
  return block;
};

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason: unknown) => void } {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("the vault gate screen", () => {
  beforeEach(() => {
    vaultStateMock.mockReset().mockResolvedValue("locked");
    vaultUnlockMock.mockReset();
    vaultSetAsideMock.mockReset();
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    while (mounted.length > 0) {
      mounted.pop()?.();
    }
    document.body.innerHTML = "";
  });

  it("draws a refused password under the field it was typed in, above the fallback", async () => {
    const root = await mountLocked();
    const input = passwordField(root);
    typeInto(input, "not the password");

    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    submit(root);
    await settle();

    const refusal = root.querySelector("#vault-unlock-error");
    expect(refusal).not.toBeNull();
    expect(refusal?.textContent).toContain("The vault did not open");
    // The message belongs beside the field it is about: at the foot of the card it would arrive
    // after the fallback block had already offered the next thing to click.
    expect(refusal?.previousElementSibling).toBe(input);
    expect(refusal!.compareDocumentPosition(fallbackBlock(root)) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-describedby")).toBe("vault-unlock-error");
  });

  it("keeps what was typed and puts the caret back in the field", async () => {
    const root = await mountLocked();
    const input = passwordField(root);
    typeInto(input, "not the password");

    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    submit(root);
    await settle();

    expect(input.value).toBe("not the password");
    expect(document.activeElement).toBe(input);
    // Selected, so retrying is one keystroke instead of a click, a select-all and a keystroke.
    expect(input.selectionStart).toBe(0);
    expect(input.selectionEnd).toBe("not the password".length);
  });

  it("draws a failed set-aside beside its own button, not under the password field", async () => {
    const root = await mountLocked();
    const block = fallbackBlock(root);
    const asideButton = block.querySelector("button");
    if (!asideButton) {
      throw new Error("the fallback block has no button");
    }

    vaultSetAsideMock.mockRejectedValue(SET_ASIDE_FAILED);
    asideButton.click();
    await settle();
    asideButton.click();
    await settle();

    const failure = root.querySelector("#vault-aside-error");
    expect(failure).not.toBeNull();
    expect(failure?.textContent).toContain("STORE-IO");
    expect(asideButton.compareDocumentPosition(failure!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The password field is not what failed, and it must not be dressed up as if it were.
    expect(root.querySelector("#vault-unlock-error")).toBeNull();
    expect(passwordField(root).getAttribute("aria-invalid")).toBeNull();
  });

  it("says what it is doing while the key is being derived", async () => {
    const root = await mountLocked();
    const input = passwordField(root);
    typeInto(input, "correct horse battery staple");

    const pending = deferred<"locked">();
    vaultUnlockMock.mockReturnValue(pending.promise);
    submit(root);
    await settle();

    const button = root.querySelector<HTMLButtonElement>("button[type='submit']");
    expect(button?.textContent).toContain("Unlocking…");
    expect(button?.querySelector(".animate-spin")).not.toBeNull();
    expect(button?.disabled).toBe(true);

    pending.resolve("locked");
    await settle();

    expect(root.querySelector<HTMLButtonElement>("button[type='submit']")?.textContent).toContain("Unlock");
  });
});
