/**
 * What the gate screen does after a button is pressed.
 *
 * The store is the real one and only the transport is mocked, because the behaviour under test is
 * the pairing between a failure and the control that caused it: the store records which operation
 * failed, and this screen is the only thing that can get the pairing wrong.
 *
 * What the screen *says* is under test here too, for the same reason: the two quiet ways out are the
 * only exits from a password nobody remembers, and one of them changes what is on disk.
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
  vaultArchives: vi.fn(),
  vaultArchiveVerify: vi.fn(),
  vaultArchiveActivate: vi.fn(),
  vaultArchiveImport: vi.fn(),
  vaultRevealFolder: vi.fn(),
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

/** One of the two quiet ways out, found by the words on it. */
function wayOut(root: HTMLElement, label: string): HTMLButtonElement {
  const button = [...root.querySelectorAll("button")].find((element) => element.textContent?.trim() === label);
  if (!button) {
    throw new Error(`the gate screen has no way out called \`${label}\``);
  }
  return button;
}

/** The dialog is teleported to the body, so it is not inside the screen's own element. */
function dialog(): HTMLElement {
  const element = document.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]');
  if (!element) {
    throw new Error("no question is on screen");
  }
  return element;
}

function dialogButton(label: string): HTMLButtonElement {
  const button = [...dialog().querySelectorAll("button")].find((element) => element.textContent?.trim() === label);
  if (!button) {
    throw new Error(`the question has no \`${label}\` button`);
  }
  return button;
}

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

  it("drops the refusal as soon as the password is edited", async () => {
    const root = await mountLocked();
    const input = passwordField(root);
    typeInto(input, "not the password");

    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    submit(root);
    await settle();
    expect(root.querySelector("#vault-unlock-error")).not.toBeNull();

    // The sentence was about that value, so it stops being true when the value changes — and the red
    // border with it. What this cannot check is the *sit* of it: that the click target and the label
    // are the same width is a layout fact, and the end-to-end run is where a real window says so.
    typeInto(input, "not the password at all");
    await settle();

    expect(root.querySelector("#vault-unlock-error"), "the stale refusal is gone").toBeNull();
    expect(input.getAttribute("aria-invalid")).toBeNull();
  });

  it("makes each way out as wide as its own words", async () => {
    const root = await mountLocked();

    for (const label of ["Forgot the password? Archive and reset", "Switch or manage archived vaults"]) {
      const link = wayOut(root, label);
      expect(link.className, "a row-wide button is a target somebody hits aiming at nothing").not.toContain("w-full");
      expect(link.className).not.toContain("block");
    }
    // And the row they sit in is a row, not something that answers a click of its own.
    expect(wayOut(root, "Forgot the password? Archive and reset").parentElement?.tagName).toBe("DIV");
  });

  it("asks for the master password, and offers the two quiet ways out under it", async () => {
    const root = await mountLocked();

    expect(root.textContent).toContain("Enter the master password to unlock the session and credential store.");
    expect(wayOut(root, "Forgot the password? Archive and reset")).toBeDefined();
    expect(wayOut(root, "Switch or manage archived vaults")).toBeDefined();

    // The old fallback block was a dashed box that caught the eye of somebody who had only mistyped;
    // what replaced it is one quiet line each.
    expect(root.querySelector(".border-dashed")).toBeNull();
    // And the footer sits below the form, never in front of it.
    expect(
      passwordField(root).compareDocumentPosition(wayOut(root, "Forgot the password? Archive and reset")) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("draws a refused password under the field it was typed in, above the ways out", async () => {
    const root = await mountLocked();
    const input = passwordField(root);
    typeInto(input, "not the password");

    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    submit(root);
    await settle();

    const refusal = root.querySelector("#vault-unlock-error");
    expect(refusal).not.toBeNull();
    expect(refusal?.textContent).toContain(
      "Unlock failed: the password is wrong, or the vault file failed verification.",
    );
    // The code stays under the sentence: it is what somebody quotes when the reason is not obvious.
    expect(refusal?.textContent).toContain("VAULT-UNLOCK-FAILED");
    // The message belongs beside the field it is about: at the foot of the card it would arrive
    // after the ways out had already offered the next thing to click. The field's own box is the
    // wrapper the input and its eye live in, so that is what the refusal follows.
    expect(refusal?.previousElementSibling?.contains(input)).toBe(true);
    expect(
      refusal!.compareDocumentPosition(wayOut(root, "Forgot the password? Archive and reset")) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-describedby")).toBe("vault-unlock-error");
  });

  it("keeps the eye on the field through a refusal, and shows what was typed", async () => {
    const root = await mountLocked();

    const eye = () => {
      const button = root.querySelector<HTMLButtonElement>("[data-vault-password-eye]");
      if (!button) {
        throw new Error("the unlock field has no show/hide control");
      }
      return button;
    };
    // It is there before anything is typed, which is the point: the field is a password field from
    // the first keystroke, and the control that reveals it is not a reward for typing one.
    expect(eye().type, "a button inside the form defaults to submit").toBe("button");

    typeInto(passwordField(root), "not the password");
    eye().click();
    await settle();
    expect(
      root.querySelector<HTMLInputElement>("#vault-password")?.getAttribute("type"),
      "and it shows what is there",
    ).toBe("text");

    // The refusal changes the border and nothing else: the eye is what somebody reaches for *after*
    // getting it wrong, so it must not be the thing that goes away when they do.
    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    submit(root);
    await settle();

    expect(passwordField(root).getAttribute("aria-invalid")).toBe("true");
    expect(eye(), "still there, still working").not.toBeNull();
    expect(root.querySelector<HTMLInputElement>("#vault-password")?.value).toBe("not the password");

    eye().click();
    await settle();
    expect(root.querySelector<HTMLInputElement>("#vault-password")?.getAttribute("type")).toBe("password");
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

  it("asks about archiving in a question of its own rather than on the button", async () => {
    const root = await mountLocked();

    wayOut(root, "Forgot the password? Archive and reset").click();
    await settle();

    const question = dialog();
    expect(question.textContent, "the title names the action").toContain("Archive the current vault?");
    expect(question.textContent, "the message says what survives").toContain("nothing is deleted");
    expect(dialogButton("Archive it")).toBeDefined();
    expect(vaultSetAsideMock, "nothing happens until the question is answered").not.toHaveBeenCalled();
  });

  it("draws a failed archive inside the question, not under the password field", async () => {
    const root = await mountLocked();

    vaultSetAsideMock.mockRejectedValue(SET_ASIDE_FAILED);
    wayOut(root, "Forgot the password? Archive and reset").click();
    await settle();
    dialogButton("Archive it").click();
    await settle();

    const question = dialog();
    expect(question.textContent).toContain("STORE-IO");
    // The question stays up: what went wrong is about the action behind it.
    expect(document.querySelector('[role="dialog"], [role="alertdialog"]')).not.toBeNull();
    // The password field is not what failed, and it must not be dressed up as if it were.
    expect(root.querySelector("#vault-unlock-error")).toBeNull();
    expect(passwordField(root).getAttribute("aria-invalid")).toBeNull();
  });

  it("goes to the create form once the unopenable vault has been archived", async () => {
    const root = await mountLocked();

    vaultSetAsideMock.mockResolvedValue("vault.json.old");
    // The state the store asks for after the rename: there is no vault in place any more.
    vaultStateMock.mockResolvedValue("missing");
    wayOut(root, "Forgot the password? Archive and reset").click();
    await settle();
    dialogButton("Archive it").click();
    await settle();

    expect(document.querySelector('[role="dialog"], [role="alertdialog"]')).toBeNull();
    expect(root.querySelector<HTMLInputElement>("#vault-confirmation")).not.toBeNull();
    // And where the data went is said, because a rename nobody witnessed looks like a deletion.
    expect(root.textContent).toContain("Kept as vault.json.old");
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
