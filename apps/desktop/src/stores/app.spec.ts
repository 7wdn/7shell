import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({ appInfo: vi.fn() }));

import { appInfo } from "@/lib/backend/tauri";
import { useAppStore } from "@/stores/app";

const appInfoMock = vi.mocked(appInfo);

const readyInfo = {
  name: "7Shell",
  version: "0.1.0",
  os: "windows",
  arch: "x86_64",
  debugBuild: true,
  protocolCount: 1,
  descriptorSchemaVersion: 1,
};

describe("app store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    appInfoMock.mockReset();
  });

  it("reports a ready backend with what it was given", async () => {
    appInfoMock.mockResolvedValue(readyInfo);

    const store = useAppStore();
    await store.load();

    expect(store.status).toBe("ready");
    expect(store.failure).toBeNull();
    expect(store.info).toMatchObject({ name: "7Shell", protocolCount: 1 });
    expect(store.platform).toBe("windows x86_64");
  });

  it("keeps the envelope when the backend fails", async () => {
    appInfoMock.mockRejectedValue({
      version: 1,
      code: "APP-NOT-READY",
      messageKey: "errors.app.notReady",
      messageParams: {},
      origin: { subsystem: "app" },
      outcome: "rejected",
    });

    const store = useAppStore();
    await store.load();

    expect(store.status).toBe("failed");
    expect(store.failure?.code).toBe("APP-NOT-READY");
    expect(store.failure?.nothingWasSent).toBe(true);
  });

  it("clears a previous failure on the next attempt", async () => {
    appInfoMock.mockRejectedValueOnce("offline").mockResolvedValue({ ...readyInfo, os: "linux" });

    const store = useAppStore();
    await store.load();
    expect(store.status).toBe("failed");

    await store.load();
    expect(store.status).toBe("ready");
    expect(store.failure).toBeNull();
    expect(store.platform).toBe("linux x86_64");
  });
});
