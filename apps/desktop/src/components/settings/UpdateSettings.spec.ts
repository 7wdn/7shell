/**
 * The update block: what the channel said, and what the page does with each answer.
 *
 * The rules of the channel itself — which release is newer, what a signature has to prove — are the
 * shell's and are tested where they live. This file is about the surface, and about the one
 * distinction the surface must never blur: "nothing newer is published" and "the channel could not
 * be read" are different sentences, and a build that drew the first for the second would be telling
 * somebody they are up to date when nobody knows.
 */
import { createPinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  updateCheck: vi.fn(),
  updateInstall: vi.fn(),
  onUpdateProgress: vi.fn(),
}));

import UpdateSettings from "@/components/settings/UpdateSettings.vue";
import { i18n } from "@/i18n";
import { onUpdateProgress, updateCheck, updateInstall } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";

const checkMock = vi.mocked(updateCheck);
const installMock = vi.mocked(updateInstall);
const progressMock = vi.mocked(onUpdateProgress);

let app: App | undefined;
let host: HTMLDivElement;

/**
 * Mounts the block on its own.
 *
 * The settings page loads the preferences before it draws anything; this block is drawn with the
 * store at its defaults, because what it does with them is this file's business and what the file
 * says is the preferences store's own spec.
 */
async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(UpdateSettings) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await nextTick();
}

/** Presses the button whose label contains this much of a word, and waits for the work. */
async function press(label: string): Promise<void> {
  const button = [...host.querySelectorAll("button")].find((candidate) => candidate.textContent?.includes(label));
  if (!button) {
    throw new Error(`no button reading ${label}`);
  }
  button.click();
  await nextTick();
  await nextTick();
}

function text(): string {
  return host.textContent ?? "";
}

beforeEach(() => {
  checkMock.mockReset();
  installMock.mockReset().mockResolvedValue(undefined);
  progressMock.mockReset().mockResolvedValue(() => {});
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the update block", () => {
  it("says a release is published, with what it changed, and offers to install it", async () => {
    checkMock.mockResolvedValue({
      kind: "available",
      currentVersion: "0.1.0-rc.1",
      release: { version: "0.1.0-rc.2", notes: "a fixed transfer that stalled", publishedAt: 1_780_000_000 },
    });
    await mount();

    await press("Check for updates");

    expect(text()).toContain("0.1.0-rc.2");
    expect(text()).toContain("a fixed transfer that stalled");
    expect(text()).toContain("Install and restart");
  });

  it("does not draw a failed check as being up to date", async () => {
    checkMock.mockRejectedValue({
      version: 1,
      code: "UPDATE-CHECK-FAILED",
      messageKey: "errors.update.checkFailed",
      messageParams: {},
      origin: { subsystem: "app" },
      outcome: "notSent",
    });
    await mount();

    await press("Check for updates");

    expect(host.querySelector("[data-update-problem]")?.textContent).toContain("UPDATE-CHECK-FAILED");
    expect(text()).not.toContain("Nothing newer");
    expect(text()).not.toContain("Install and restart");
  });

  it("says what a development build is instead of pretending to check", async () => {
    checkMock.mockResolvedValue({ kind: "notInstalled", currentVersion: "0.1.0" });
    await mount();

    await press("Check for updates");

    expect(text()).toContain("development build");
    expect(text()).not.toContain("Install and restart");
  });

  it("shows how far a download has got while it runs", async () => {
    checkMock.mockResolvedValue({
      kind: "available",
      currentVersion: "0.1.0-rc.1",
      release: { version: "0.1.0-rc.2" },
    });
    let listener: ((payload: { kind: "downloading"; downloaded: number; total?: number }) => void) | undefined;
    progressMock.mockImplementation(async (register) => {
      listener = register as typeof listener;
      return () => {};
    });
    installMock.mockImplementation(() => new Promise<void>(() => {}));
    await mount();

    await press("Check for updates");
    await press("Install and restart");
    listener?.({ kind: "downloading", downloaded: 3 * 1024 * 1024, total: 6 * 1024 * 1024 });
    await nextTick();

    const progress = host.querySelector("[data-update-progress]")?.textContent ?? "";
    expect(progress).toContain("3.0 MB");
    expect(progress).toContain("6.0 MB");
  });

  it("writes the startup switch through the settings the page shares", async () => {
    await mount();
    const preferences = usePreferencesStore();

    expect(preferences.updateCheckOnStartup).toBe(true);
    const toggle = host.querySelector<HTMLElement>('[role="switch"]');
    if (!toggle) {
      throw new Error("the startup switch is not on the page");
    }
    toggle.click();
    await nextTick();

    expect(preferences.updateCheckOnStartup).toBe(false);
  });
});
