/**
 * The backup that goes out on its own.
 *
 * What is worth pinning here is everything that decides *whether* a request leaves: a locked vault,
 * a switch that is off, a tick arriving while the last upload is still running, and the line between
 * credentials travelling and credentials staying. The upload itself is tested where it lives.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  syncStatus: vi.fn(),
  syncUpload: vi.fn(),
  syncBackupExport: vi.fn(),
  syncBackupInspect: vi.fn(),
  syncBackupRestore: vi.fn(),
  syncAvailable: vi.fn(),
}));

import { useSyncAutoUpload } from "@/composables/useSyncAutoUpload";
import { useToast } from "@/composables/useToast";
import { i18n } from "@/i18n";
import { ShellError } from "@/lib/backend/errors";
import { syncStatus, syncUpload } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useVaultStore } from "@/stores/vault";
import type { SyncUploadReceipt } from "@/types/sync";

const statusMock = vi.mocked(syncStatus);
const uploadMock = vi.mocked(syncUpload);

let app: App | undefined;
let host: HTMLDivElement;

const receipt: SyncUploadReceipt = {
  contents: {
    sessions: 1,
    groups: 0,
    snippets: 0,
    words: 0,
    history: 0,
    knownHosts: 0,
    settings: 18,
    secrets: 0,
    keys: 0,
  },
  sealed: false,
  remotePath: "7shell/sync/backup.json",
};

/** Mounts the frame's own use of the composable, which is where it is called in the application. */
function mount(): void {
  host = document.createElement("div");
  app = createApp({
    setup() {
      useSyncAutoUpload();
      return () => h("div");
    },
  })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
}

/** A vault that is open, a server that is configured, and the timer switched on. */
async function armed(options: { includeSecrets?: boolean; hasPassphrase?: boolean } = {}): Promise<void> {
  const vault = useVaultStore();
  const preferences = usePreferencesStore();
  vault.state = "unlocked";
  preferences.syncEndpoint = "https://dav.example.com/dav/";
  preferences.syncAutoUpload = true;
  preferences.syncAutoUploadMinutes = 30;
  preferences.syncIncludeSecrets = options.includeSecrets === true;
  statusMock.mockResolvedValue({ hasPassword: true, hasPassphrase: options.hasPassphrase === true });
  await nextTick();
}

/** Runs time forward the way a timer does, and lets the awaits inside it finish. */
async function tickThatArrives(minutes = 30): Promise<void> {
  await vi.advanceTimersByTimeAsync(minutes * 60_000);
  await nextTick();
}

beforeEach(() => {
  vi.useFakeTimers();
  setActivePinia(createPinia());
  statusMock.mockReset().mockResolvedValue({ hasPassword: false, hasPassphrase: false });
  uploadMock.mockReset().mockResolvedValue(receipt);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  vi.useRealTimers();
});

describe("the timed backup", () => {
  it("sends nothing while the vault is locked", async () => {
    mount();
    usePreferencesStore().syncEndpoint = "https://dav.example.com/dav/";
    usePreferencesStore().syncAutoUpload = true;
    await nextTick();

    await tickThatArrives();

    expect(uploadMock, "the library and every secret are behind the vault").not.toHaveBeenCalled();
  });

  it("sends nothing while the switch is off, and something once it is on", async () => {
    mount();
    const vault = useVaultStore();
    const preferences = usePreferencesStore();
    vault.state = "unlocked";
    preferences.syncEndpoint = "https://dav.example.com/dav/";
    await nextTick();

    await tickThatArrives();
    expect(uploadMock, "nobody asked for a timer").not.toHaveBeenCalled();

    preferences.syncAutoUpload = true;
    await tickThatArrives();

    expect(uploadMock).toHaveBeenCalledTimes(1);
  });

  it("drops a tick that arrives while the last one is still running", async () => {
    // A slow server must not turn into a queue of uploads: the tick that cannot start is the one
    // that should be forgotten, not the one that should wait.
    let release: (() => void) | undefined;
    uploadMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () => resolve(receipt);
        }),
    );
    mount();
    await armed();

    await tickThatArrives();
    await tickThatArrives();
    await tickThatArrives();

    expect(uploadMock, "one at a time").toHaveBeenCalledTimes(1);
    release?.();
    await tickThatArrives();
    expect(uploadMock, "and the next tick after it finishes is taken").toHaveBeenCalledTimes(2);
  });

  it("carries the credentials only when the switch asks and a passphrase is kept", async () => {
    mount();
    await armed({ includeSecrets: true, hasPassphrase: false });

    await tickThatArrives();

    expect(uploadMock.mock.calls[0]?.[0]).toMatchObject({ credentials: false });

    uploadMock.mockClear();
    await armed({ includeSecrets: true, hasPassphrase: true });
    await tickThatArrives();

    expect(uploadMock.mock.calls[0]?.[0]).toMatchObject({ credentials: true });
    expect(uploadMock.mock.calls[0]?.[1], "and the kept passphrase is what seals them").toBeNull();
  });

  it("says a failure out loud and says nothing about a success", async () => {
    mount();
    await armed();

    await tickThatArrives();
    expect(useToast().message.value, "a backup that worked is nobody's news").toBe("");

    uploadMock.mockRejectedValue(
      new ShellError({
        version: 1,
        code: "SYNC-UNREACHABLE",
        messageKey: "errors.sync.unreachable",
        messageParams: {},
        origin: { subsystem: "net" },
        outcome: "notSent",
      }),
    );
    await tickThatArrives();

    expect(useToast().message.value, "a backup that failed has no place on screen").toContain("could not be reached");
  });
});
