/**
 * The release entry in the title bar, as a person meets it.
 *
 * Two things about each of its states: what it *says* — the tooltip is the only place an icon can
 * say "checking" rather than "check", or that the channel could not be read — and that pressing it
 * opens the window where the answer is looked at. It deliberately does not ask the channel itself:
 * that window asks as it opens, and two asks for one click would be two answers.
 *
 * A development build is deliberately not covered: there is no installed copy for an update to
 * replace, and the entry is not drawn at all in a browser.
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

import UpdateEntry from "@/components/shell/UpdateEntry.vue";
import { i18n } from "@/i18n";
import { updateCheck } from "@/lib/backend/tauri";
import { useUpdatesStore } from "@/stores/updates";

const checkMock = vi.mocked(updateCheck);

/** What the channel answers when it offers a release. */
const published = {
  kind: "available" as const,
  currentVersion: "0.1.0",
  release: { version: "0.1.1", notes: "the second candidate", publishedAt: 1_780_000_000 },
};

/** Mounts the entry on its own and hands back what the document then holds. */
function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(UpdateEntry).use(createPinia()).use(i18n);
  app.mount(host);
  return { host, unmount: () => app.unmount() };
}

const entry = (host: HTMLElement): HTMLButtonElement | null => host.querySelector("[data-update-entry]");

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("the release entry", () => {
  it("is there before anything is known, and opens the window where the answer is", async () => {
    const { host, unmount } = mount();
    const updates = useUpdatesStore();

    const button = entry(host);
    expect(button, "the entry is there before there is an answer").not.toBeNull();
    expect(button?.dataset.updateAvailable, "nothing is published yet").toBeUndefined();
    expect(button?.title).toBe("Check for updates");

    button?.click();
    await nextTick();

    expect(updates.dialogOpen, "the window about the release is what it opens").toBe(true);
    unmount();
  });

  it("carries the version once a release is published", async () => {
    const { host, unmount } = mount();
    const updates = useUpdatesStore();
    checkMock.mockResolvedValue(published);
    await updates.check();
    await nextTick();

    const button = entry(host);
    expect(button?.dataset.updateAvailable, "a published release is what the mark means").toBe("");
    expect(host.querySelector("[data-update-version]")?.textContent?.trim()).toBe("0.1.1");
    expect(button?.title).toBe("Update to 0.1.1");

    button?.click();
    await nextTick();
    expect(updates.dialogOpen).toBe(true);
    unmount();
  });

  it("says it is checking while it is", async () => {
    let answer: (value: unknown) => void = () => {};
    checkMock.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve as (value: unknown) => void;
      }) as ReturnType<typeof updateCheck>,
    );
    const { host, unmount } = mount();
    const updates = useUpdatesStore();

    const asking = updates.check();
    await nextTick();

    expect(entry(host)?.title, "an icon cannot say this by itself").toBe("Checking…");

    answer({ kind: "upToDate", currentVersion: "0.1.0" });
    await asking;
    await nextTick();

    expect(entry(host)?.title).toBe("Check for updates");
    unmount();
  });

  it("says so when the channel could not be read, rather than looking up to date", async () => {
    const { host, unmount } = mount();
    const updates = useUpdatesStore();
    checkMock.mockRejectedValue({
      version: 1,
      code: "UPDATE-CHECK-FAILED",
      messageKey: "errors.update.checkFailed",
      origin: { subsystem: "app" },
    });

    await updates.check();
    await nextTick();

    expect(entry(host)?.title).toBe("The update channel could not be read");
    unmount();
  });
});
