/**
 * The vault, as the security page draws it.
 *
 * Two behaviours, and both are about a rule being visible before it is broken: the change form
 * refuses to submit while the two new passwords disagree or the new one is the current one, and a
 * refusal from the backend is drawn under the form that caused it rather than swallowed. The third
 * thing here is the lock: it is the one control in the interface that shuts the vault on purpose, and
 * what it leaves behind is the gate.
 */
import { createPinia, setActivePinia } from "pinia";
import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  vaultState: vi.fn(),
  vaultCreate: vi.fn(),
  vaultChangePassword: vi.fn(),
  vaultLock: vi.fn(),
  vaultArchives: vi.fn(),
  vaultArchiveVerify: vi.fn(),
  vaultArchiveActivate: vi.fn(),
  vaultArchiveImport: vi.fn(),
  vaultRevealFolder: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
}));

import VaultSettings from "@/components/settings/VaultSettings.vue";
import { i18n } from "@/i18n";
import {
  groupsList,
  sessionsList,
  vaultArchives,
  vaultChangePassword,
  vaultLock,
  vaultState,
} from "@/lib/backend/tauri";
import { useVaultStore } from "@/stores/vault";

const vaultStateMock = vi.mocked(vaultState);
const changeMock = vi.mocked(vaultChangePassword);
const lockMock = vi.mocked(vaultLock);
const archivesMock = vi.mocked(vaultArchives);

const WRONG_CURRENT = {
  version: 1,
  code: "VAULT-UNLOCK-FAILED",
  messageKey: "errors.vault.unlockFailed",
  messageParams: {},
  origin: { subsystem: "vault" },
  outcome: "rejected",
};

const mounted: Array<() => void> = [];

async function mount(): Promise<HTMLElement> {
  vaultStateMock.mockResolvedValue("unlocked");

  const pinia = createPinia();
  setActivePinia(pinia);
  await useVaultStore().initialize();

  const app = createApp(VaultSettings);
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
  return root;
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
}

function field(root: HTMLElement, id: string, value: string): void {
  const input = root.querySelector<HTMLInputElement>(`#${id}`);
  if (!input) {
    throw new Error(`the form has no ${id} field`);
  }
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

function button(root: HTMLElement, label: string): HTMLButtonElement {
  const found = [...root.querySelectorAll("button")].find((element) => element.textContent?.trim() === label);
  if (!found) {
    throw new Error(`there is no \`${label}\` button`);
  }
  return found;
}

describe("the vault block on the security page", () => {
  beforeEach(() => {
    vaultStateMock.mockReset().mockResolvedValue("unlocked");
    changeMock.mockReset();
    lockMock.mockReset().mockResolvedValue("locked");
    archivesMock.mockReset().mockResolvedValue([]);
    vi.mocked(sessionsList).mockReset().mockResolvedValue([]);
    vi.mocked(groupsList).mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    while (mounted.length > 0) {
      mounted.pop()?.();
    }
    document.body.innerHTML = "";
  });

  it("holds the change back while the two new passwords disagree", async () => {
    const root = await mount();

    field(root, "vault-current-password", "the old one");
    field(root, "vault-next-password", "a new one entirely");
    field(root, "vault-next-confirmation", "a new one entir");
    await settle();

    expect(root.textContent).toContain("The two passwords do not match");
    expect(button(root, "Change the master password").disabled).toBe(true);
    expect(changeMock).not.toHaveBeenCalled();
  });

  it("says so while the new password is still too short, and while it is the current one", async () => {
    const root = await mount();

    field(root, "vault-current-password", "the old one");
    field(root, "vault-next-password", "short");
    await settle();
    expect(root.textContent).toContain("Use at least 8 characters");
    expect(button(root, "Change the master password").disabled).toBe(true);

    field(root, "vault-next-password", "the old one");
    field(root, "vault-next-confirmation", "the old one");
    await settle();
    expect(root.textContent).toContain("The new password is the current one");
    expect(button(root, "Change the master password").disabled).toBe(true);
    expect(changeMock).not.toHaveBeenCalled();
  });

  it("changes the password, clears every field and says the file is different now", async () => {
    const root = await mount();
    changeMock.mockResolvedValue(undefined);

    field(root, "vault-current-password", "the old one");
    field(root, "vault-next-password", "a new one entirely");
    field(root, "vault-next-confirmation", "a new one entirely");
    await settle();
    button(root, "Change the master password").click();
    await settle();

    expect(changeMock).toHaveBeenCalledWith("the old one", "a new one entirely");
    expect(root.textContent).toContain("The vault now opens with the new password");
    // Nothing that was typed stays on screen, in either of the two fields that held a password.
    expect(root.querySelector<HTMLInputElement>("#vault-current-password")?.value).toBe("");
    expect(root.querySelector<HTMLInputElement>("#vault-next-password")?.value).toBe("");
  });

  it("draws a refused change under the form, with the code that came back", async () => {
    const root = await mount();
    changeMock.mockRejectedValue(WRONG_CURRENT);

    field(root, "vault-current-password", "not the old one");
    field(root, "vault-next-password", "a new one entirely");
    field(root, "vault-next-confirmation", "a new one entirely");
    await settle();
    button(root, "Change the master password").click();
    await settle();

    const refusal = root.querySelector('[role="alert"]');
    expect(refusal?.textContent).toContain("VAULT-UNLOCK-FAILED");
    // The fields keep what was typed, so a mistyped current password is corrected rather than retyped.
    expect(root.querySelector<HTMLInputElement>("#vault-current-password")?.value).toBe("not the old one");
    expect(root.querySelector<HTMLInputElement>("#vault-next-password")?.value).toBe("a new one entirely");
  });

  it("locks the vault from here, which is what puts the gate back up", async () => {
    const root = await mount();

    button(root, "Lock now").click();
    await settle();

    expect(lockMock).toHaveBeenCalled();
    expect(useVaultStore().state).toBe("locked");
  });
});
