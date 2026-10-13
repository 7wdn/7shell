/**
 * The update centre, as a person meets it.
 *
 * Five things a state has to be honest about, and each of them is checked here: what it says about
 * the release, whether the notes are drawn as prose rather than as markup, which actions the footer
 * offers — downloading and installing are two different decisions — and that ignoring a version is
 * written down *and* takes the release off the screen.
 *
 * The shell's side of it — the archive, the signature, the installer — is not this file's business;
 * what the window does with an answer is.
 */
import { createPinia, setActivePinia } from "pinia";
import { createApp, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  updateCheck: vi.fn(),
  updateDownload: vi.fn(),
  updateInstall: vi.fn(),
  updateOpenReleasePage: vi.fn(),
  onUpdateProgress: vi.fn(async () => () => {}),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(async () => {}),
}));

import UpdateCenter from "@/components/shell/UpdateCenter.vue";
import { i18n } from "@/i18n";
import { settingsSet, updateCheck, updateDownload, updateOpenReleasePage } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useUpdatesStore } from "@/stores/updates";

const checkMock = vi.mocked(updateCheck);
const downloadMock = vi.mocked(updateDownload);
const openPageMock = vi.mocked(updateOpenReleasePage);
const settingsSetMock = vi.mocked(settingsSet);

/** What the channel answers when it offers a release, with notes worth drawing. */
const published = {
  kind: "available" as const,
  currentVersion: "0.1.0",
  release: {
    version: "0.1.1",
    notes: "### Fixed\n\n- **the terminal ignored its font settings**\n- and `style-src` was left alone\n",
    publishedAt: 1_780_000_000,
  },
};

/** Mounts the centre open, on its own. */
function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(UpdateCenter).use(createPinia()).use(i18n);
  app.mount(host);
  const updates = useUpdatesStore();
  updates.dialogOpen = true;
  return { updates, unmount: () => app.unmount() };
}

/**
 * Looks for something the window drew.
 *
 * In the document rather than in the mount point: a dialog is drawn through a portal at the top of the
 * body, which is what keeps it over everything else in the window — and what a case would miss if it
 * only looked where it mounted.
 */
const find = (selector: string): HTMLElement | null => document.body.querySelector(selector);

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  // The window asks as it opens, so every case starts from an answer.
  mockedPreferences();
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

/** A preferences store that will not reach the backend on its own. */
function mockedPreferences() {
  settingsSetMock.mockResolvedValue(undefined);
}

/**
 * Waits for the window's own check to land.
 *
 * The window asks as it opens and the answer arrives on the store's own schedule, so a case that
 * asserted straight after mounting would be reading the state before the question was asked.
 */
async function answered(updates: ReturnType<typeof useUpdatesStore>): Promise<void> {
  await vi.waitFor(() => {
    if (updates.answer === null) {
      throw new Error("the window has not been answered yet");
    }
  });
  await nextTick();
}

describe("the update centre", () => {
  it("opens by asking, and says the build is newest when nothing is published", async () => {
    checkMock.mockResolvedValue({ kind: "upToDate", currentVersion: "0.1.0" });
    const { unmount, updates } = mount();
    await answered(updates);

    expect(checkMock, "the window asks as it opens").toHaveBeenCalled();
    expect(find("[data-update-headline]")?.textContent).toContain("0.1.0");
    expect(find("[data-update-badge]"), "there is nothing to badge").toBeNull();
    expect(find("[data-update-download]")).toBeNull();
    expect(find("[data-update-restart]")).toBeNull();
    expect(find("[data-update-recheck]"), "asking by hand is still there").not.toBeNull();
    unmount();
  });

  it("says what is published over what is running, with the notes drawn as prose", async () => {
    checkMock.mockResolvedValue(published);
    const { unmount, updates } = mount();
    await answered(updates);

    expect(find("[data-update-headline]")?.textContent).toContain("0.1.1");
    expect(find("[data-update-headline]")?.textContent).toContain("0.1.0");
    expect(find("[data-update-badge]")).not.toBeNull();

    // The notes are blocks now, not one run of text: a bullet is a bullet, and a bolded phrase is
    // its own element — which is also the check that nothing arrives as markup.
    const notes = find("[data-update-notes]");
    expect(notes?.querySelectorAll("li")).toHaveLength(2);
    expect(notes?.querySelector("strong")).not.toBeNull();
    expect(notes?.querySelector("code")).not.toBeNull();
    expect(notes?.querySelector("script"), "a release body is text, never markup").toBeNull();
    unmount();
  });

  it("offers downloading and installing as two different decisions", async () => {
    checkMock.mockResolvedValue(published);
    downloadMock.mockResolvedValue(undefined);
    const { unmount, updates } = mount();
    await answered(updates);

    const download = find("[data-update-download]") as HTMLButtonElement | null;
    expect(download, "the archive is not fetched until somebody says so").not.toBeNull();
    expect(find("[data-update-restart]"), "the window is not replaced before then").toBeNull();

    download?.click();
    await nextTick();

    expect(downloadMock).toHaveBeenCalledTimes(1);
    unmount();
  });

  it("offers the restart once the archive is waiting, and not the download again", async () => {
    checkMock.mockResolvedValue(published);
    const { unmount, updates } = mount();
    await answered(updates);

    updates.downloadedVersion = "0.1.1";
    await nextTick();

    expect(find("[data-update-ready]")?.textContent).toContain("0.1.1");
    expect(find("[data-update-restart]")).not.toBeNull();
    expect(find("[data-update-download]")).toBeNull();
    unmount();
  });

  it("writes an ignored version down and stops mentioning it", async () => {
    checkMock.mockResolvedValue(published);
    const { unmount, updates } = mount();
    const preferences = usePreferencesStore();
    await preferences.initialize();
    await answered(updates);

    find("[data-update-ignore]")?.click();
    await nextTick();
    await nextTick();

    expect(settingsSetMock, "the choice reaches the file").toHaveBeenCalledWith(
      expect.objectContaining({ ignoredUpdateVersion: "0.1.1" }),
    );
    expect(updates.release, "and the release stops being this build's business").toBeNull();
    unmount();
  });

  it("opens the page a release is published on", async () => {
    checkMock.mockResolvedValue(published);
    openPageMock.mockResolvedValue(undefined);
    const { unmount, updates } = mount();
    await answered(updates);

    find("[data-update-page]")?.click();
    await nextTick();

    expect(openPageMock).toHaveBeenCalledWith("0.1.1");
    unmount();
  });
});
