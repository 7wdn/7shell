/**
 * The release entry in the title bar, as a person meets it.
 *
 * Three states and the two things that matter about each: what it *says* — the tooltip is the only
 * place an icon can say "checking" rather than "check" — and what pressing it does. The store's own
 * suite has the channel; what is checked here is that the entry reads it.
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
  updateInstall: vi.fn(),
  onUpdateProgress: vi.fn(async () => () => {}),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(async () => {}),
  sessionsList: vi.fn(async () => []),
  groupsList: vi.fn(async () => []),
}));

import UpdateEntry from "@/components/shell/UpdateEntry.vue";
import { i18n } from "@/i18n";
import { updateCheck } from "@/lib/backend/tauri";
import { useTerminalsStore } from "@/stores/terminals";
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
  it("offers to ask before anything is known, and asking is what it does", async () => {
    checkMock.mockResolvedValue(published);
    const { host, unmount } = mount();

    const button = entry(host);
    expect(button, "the entry is there before there is an answer").not.toBeNull();
    expect(button?.dataset.updateAvailable, "nothing is published yet").toBeUndefined();
    expect(button?.title).toBe("Check for updates");

    button?.click();
    await nextTick();

    expect(checkMock).toHaveBeenCalledTimes(1);
    unmount();
  });

  it("carries the version once a release is published, and leads to the page that installs it", async () => {
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

    const terminals = useTerminalsStore();
    expect(terminals.settingsRequest, "the page is asked to open where the update lives").toBe("about");
    expect(terminals.activeTab?.kind, "and the page it leads to is open").toBe("settings");
    unmount();
  });

  it("says it is checking while it is, and cannot be asked twice", async () => {
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

    const button = entry(host);
    expect(button?.title, "an icon cannot say this by itself").toBe("Checking…");
    expect(button?.disabled, "a second ask while the first is out is nothing to offer").toBe(true);

    answer({ kind: "up-to-date", currentVersion: "0.1.0" });
    await asking;
    await nextTick();

    expect(entry(host)?.disabled).toBe(false);
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
