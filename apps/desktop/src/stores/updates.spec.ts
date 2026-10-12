import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  updateCheck: vi.fn(),
  updateInstall: vi.fn(),
  onUpdateProgress: vi.fn(),
}));

import { onUpdateProgress, updateCheck, updateInstall } from "@/lib/backend/tauri";
import { useUpdatesStore } from "@/stores/updates";
import type { UpdateProgress } from "@/types/update";

const checkMock = vi.mocked(updateCheck);
const installMock = vi.mocked(updateInstall);
const progressMock = vi.mocked(onUpdateProgress);

/** What the channel answers when it offers a release. */
const published = {
  kind: "available" as const,
  currentVersion: "0.1.0-rc.1",
  release: { version: "0.1.0-rc.2", notes: "the second candidate", publishedAt: 1_780_000_000 },
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  progressMock.mockResolvedValue(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the update channel as the interface reads it", () => {
  it("holds the release a check found, and the version it was found against", async () => {
    checkMock.mockResolvedValue(published);
    const updates = useUpdatesStore();

    await updates.check();

    expect(updates.release?.version).toBe("0.1.0-rc.2");
    expect(updates.answer?.currentVersion).toBe("0.1.0-rc.1");
    expect(updates.failure).toBeNull();
    expect(updates.asking).toBe(false);
  });

  it("keeps a failed check apart from an up-to-date answer", async () => {
    // The one distinction that matters on this page: "the channel could not be read" must never be
    // drawn as "you are running the newest release", because only one of those is a fact.
    checkMock.mockRejectedValue({
      version: 1,
      code: "UPDATE-CHECK-FAILED",
      messageKey: "errors.update.checkFailed",
      origin: { subsystem: "app" },
    });
    const updates = useUpdatesStore();

    await updates.check();

    expect(updates.failure?.code).toBe("UPDATE-CHECK-FAILED");
    expect(updates.answer).toBeNull();
    expect(updates.release).toBeNull();
  });

  it("drops a release the channel no longer offers", async () => {
    checkMock.mockResolvedValueOnce(published);
    const updates = useUpdatesStore();
    await updates.check();
    expect(updates.release).not.toBeNull();

    // A release that was published and then withdrawn — or one this build has caught up with — is
    // not installable any more, and a page that kept offering it would be offering nothing.
    checkMock.mockResolvedValueOnce({ kind: "upToDate", currentVersion: "0.1.0-rc.1" });
    await updates.check();

    expect(updates.release).toBeNull();
    expect(updates.answer?.kind).toBe("upToDate");
  });

  it("carries the development build's answer as an answer, not as a failure", async () => {
    checkMock.mockResolvedValue({ kind: "notInstalled", currentVersion: "0.1.0" });
    const updates = useUpdatesStore();

    await updates.check();

    expect(updates.answer?.kind).toBe("notInstalled");
    expect(updates.failure).toBeNull();
  });

  it("shows how far an install has got, and stops listening when it is over", async () => {
    checkMock.mockResolvedValue(published);
    // The install never comes back: on Windows the installer takes the process over, and on macOS
    // and Linux the application restarts into the new version. The store has to be able to say what
    // it was doing while it did not come back.
    let listener: ((payload: UpdateProgress) => void) | undefined;
    progressMock.mockImplementation(async (register) => {
      listener = register;
      return () => {
        listener = undefined;
      };
    });
    installMock.mockImplementation(
      () =>
        new Promise<void>(() => {
          // Never settles, which is what a successful install looks like from here.
        }),
    );

    const updates = useUpdatesStore();
    await updates.check();
    void updates.install();
    await Promise.resolve();

    listener?.({ kind: "downloading", downloaded: 2 * 1024 * 1024, total: 8 * 1024 * 1024 });
    expect(updates.progress).toEqual({ kind: "downloading", downloaded: 2 * 1024 * 1024, total: 8 * 1024 * 1024 });
    expect(updates.installing).toBe(true);

    listener?.({ kind: "installing" });
    expect(updates.progress).toEqual({ kind: "installing" });
  });

  it("reports an install that failed", async () => {
    checkMock.mockResolvedValue(published);
    installMock.mockRejectedValue({
      version: 1,
      code: "UPDATE-INSTALL-FAILED",
      messageKey: "errors.update.installFailed",
      origin: { subsystem: "app" },
    });
    const updates = useUpdatesStore();

    await updates.check();
    await updates.install();

    expect(updates.failure?.code).toBe("UPDATE-INSTALL-FAILED");
    expect(updates.installing).toBe(false);
  });
});
