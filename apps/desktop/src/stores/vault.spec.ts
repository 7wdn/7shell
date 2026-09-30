import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  vaultState: vi.fn(),
  vaultCreate: vi.fn(),
  vaultUnlock: vi.fn(),
  vaultLock: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
}));

import { groupsList, sessionsList, vaultCreate, vaultLock, vaultState, vaultUnlock } from "@/lib/backend/tauri";
import { useSessionsStore } from "@/stores/sessions";
import { useVaultStore } from "@/stores/vault";

const vaultStateMock = vi.mocked(vaultState);
const vaultCreateMock = vi.mocked(vaultCreate);
const vaultUnlockMock = vi.mocked(vaultUnlock);
const vaultLockMock = vi.mocked(vaultLock);
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

const session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
  updatedAt: 0,
} as const;

describe("vault store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vaultStateMock.mockReset().mockResolvedValue("locked");
    vaultCreateMock.mockReset().mockResolvedValue("unlocked");
    vaultUnlockMock.mockReset().mockResolvedValue("unlocked");
    vaultLockMock.mockReset().mockResolvedValue("locked");
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
  });

  it("asks the backend where the vault stands instead of assuming", async () => {
    vaultStateMock.mockResolvedValue("missing");

    const store = useVaultStore();
    await store.initialize();

    expect(store.state).toBe("missing");
    expect(store.unlocked).toBe(false);
    // Nothing to load: asking for sessions while there is no vault would only produce a failure.
    expect(sessionsListMock).not.toHaveBeenCalled();
  });

  it("loads the session library when the vault is already open", async () => {
    vaultStateMock.mockResolvedValue("unlocked");
    sessionsListMock.mockResolvedValue([{ ...session }]);

    const store = useVaultStore();
    await store.initialize();

    expect(store.unlocked).toBe(true);
    expect(useSessionsStore().sessions).toHaveLength(1);
  });

  it("creates a vault and is unlocked afterwards", async () => {
    const store = useVaultStore();
    await store.initialize();

    const created = await store.create("correct horse battery staple");

    expect(created).toBe(true);
    expect(store.state).toBe("unlocked");
    expect(store.failure).toBeNull();
    expect(vaultCreateMock).toHaveBeenCalledWith("correct horse battery staple");
    expect(sessionsListMock).toHaveBeenCalledTimes(1);
  });

  it("surfaces the envelope's code when a password is refused", async () => {
    const store = useVaultStore();
    await store.initialize();

    vaultUnlockMock.mockRejectedValue(WRONG_PASSWORD);
    const opened = await store.unlock("not the password");

    expect(opened).toBe(false);
    expect(store.state).toBe("locked");
    expect(store.failure?.code).toBe("VAULT-UNLOCK-FAILED");
    expect(store.failure?.messageKey).toBe("errors.vault.unlockFailed");
    expect(store.busy).toBe(false);
  });

  it("keeps the gate unknown when the state cannot be read at all", async () => {
    vaultStateMock.mockRejectedValue("the pipe is closed");

    const store = useVaultStore();
    await store.initialize();

    expect(store.state).toBe("unknown");
    expect(store.failure?.code).toBe("APP-INTERNAL");
  });

  it("locks back to locked", async () => {
    const store = useVaultStore();
    await store.initialize();
    await store.unlock("correct horse battery staple");
    expect(store.unlocked).toBe(true);

    await store.lock();

    expect(store.state).toBe("locked");
    expect(store.unlocked).toBe(false);
  });
});
