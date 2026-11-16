/**
 * The bookmark popover, as a person meets it: opening the list, adding the directory on screen,
 * renaming an entry and removing one.
 *
 * The rules themselves — the cap, the unique name, the trimming — are `lib/files/bookmarks`'s
 * business; what can only be seen here is the wiring: that the draft starts from the directory the
 * panel is showing, that saving reaches the store, and that a rule broken while typing is drawn
 * before the Save button is worth pressing.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  appInfo: vi.fn(),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(async () => {}),
  systemTheme: vi.fn(async () => null),
  onSystemThemeChanged: vi.fn(async () => () => {}),
  windowThemeApply: vi.fn(async () => {}),
  webviewSetZoom: vi.fn(async () => {}),
}));

import FileBookmarksMenu from "@/components/files/FileBookmarksMenu.vue";
import { i18n } from "@/i18n";
import { settingsSet } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";

const settingsSetMock = vi.mocked(settingsSet);

const PATH = "/usr/local/nginx/conf";

let host: HTMLDivElement;
let app: App | undefined;
let jumps: string[];

async function render(path = PATH): Promise<void> {
  jumps = [];
  app = createApp({
    render: () =>
      h(FileBookmarksMenu, {
        path,
        onJump: (to: string) => jumps.push(to),
      }),
  }).use(i18n);
  app.mount(host);
  await nextTick();
}

/** A click followed by the two ticks Vue needs to draw whatever the click opened. */
async function press(selector: string): Promise<HTMLElement> {
  const target = document.querySelector<HTMLElement>(selector);
  if (!target) throw new Error(`nothing on screen matches ${selector}`);
  target.click();
  await nextTick();
  await nextTick();
  return target;
}

async function openMenu(): Promise<void> {
  await press("[data-bookmark-menu]");
}

function dialog(): HTMLElement {
  const found = document.querySelector<HTMLElement>("[data-bookmark-dialog]");
  if (!found) throw new Error("no bookmark dialog is on screen");
  return found;
}

/** Types into the dialog's field at `index`: the name first, the path second. */
async function type(index: number, value: string): Promise<void> {
  const input = dialog().querySelectorAll("input")[index];
  if (!input) throw new Error("the dialog does not have that field");
  input.value = value;
  input.dispatchEvent(new Event("input"));
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  settingsSetMock.mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  document.querySelector("[data-file-bookmarks]")?.remove();
  document.querySelector("[data-bookmark-dialog]")?.parentElement?.remove();
});

describe("the bookmark list", () => {
  it("lists the store's entries and jumps to the one clicked", async () => {
    await usePreferencesStore().addFileBookmark({ label: "logs", path: "/var/log" });
    await render();

    await openMenu();

    expect(document.querySelector("[data-file-bookmarks]")?.textContent).toContain("logs");
    expect(document.querySelector("[data-file-bookmarks]")?.textContent).toContain("/var/log");

    await press('[data-bookmark-action="jump"]');

    expect(jumps).toEqual(["/var/log"]);
  });

  it("says the list is empty rather than opening a blank box", async () => {
    await render();

    await openMenu();

    expect(document.querySelector("[data-file-bookmarks]")?.textContent).toContain("No bookmarks yet.");
  });

  it("copies the path of an entry", async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(window.navigator, "clipboard", { value: { writeText }, configurable: true });
    await usePreferencesStore().addFileBookmark({ label: "logs", path: "/var/log" });
    await render();

    await openMenu();
    await press('[data-bookmark-action="copy"]');

    expect(writeText).toHaveBeenCalledWith("/var/log");
  });
});

describe("adding a bookmark", () => {
  it("starts from the directory on screen, named after its last segment", async () => {
    await render();

    await openMenu();
    await press("[data-bookmark-add]");

    const inputs = dialog().querySelectorAll("input");
    expect((inputs[0] as HTMLInputElement).value).toBe("conf");
    expect((inputs[1] as HTMLInputElement).value).toBe(PATH);
  });

  it("saves through the store so the list is written down at once", async () => {
    await render();

    await openMenu();
    await press("[data-bookmark-add]");
    await press("[data-bookmark-save]");

    const bookmarks = usePreferencesStore().fileBookmarks;
    expect(bookmarks).toMatchObject([{ label: "conf", path: PATH }]);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ fileBookmarks: bookmarks }));
  });

  it("draws the broken rule while it is being typed and keeps Save out of reach", async () => {
    await usePreferencesStore().addFileBookmark({ label: "conf", path: "/etc/nginx" });
    await render();

    await openMenu();
    await press("[data-bookmark-add]");
    await type(0, "CONF");

    expect(dialog().querySelector('[role="alert"]')?.textContent).toContain(
      "A bookmark with this name already exists.",
    );
    expect(dialog().querySelector<HTMLButtonElement>("[data-bookmark-save]")?.disabled).toBe(true);
  });
});

describe("editing a bookmark", () => {
  it("renames an entry without making a second one", async () => {
    await usePreferencesStore().addFileBookmark({ label: "logs", path: "/var/log" });
    await render();

    await openMenu();
    await press('[data-bookmark-action="edit"]');
    await type(0, "Logs");
    await press("[data-bookmark-save]");

    expect(usePreferencesStore().fileBookmarks).toMatchObject([{ label: "Logs", path: "/var/log" }]);
  });
});

describe("removing a bookmark", () => {
  it("takes the entry out of the list and writes the list down", async () => {
    await usePreferencesStore().addFileBookmark({ label: "logs", path: "/var/log" });
    await render();

    await openMenu();
    await press('[data-bookmark-action="remove"]');

    expect(usePreferencesStore().fileBookmarks).toEqual([]);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ fileBookmarks: [] }));
  });
});
