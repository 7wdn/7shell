/**
 * The settings page as a surface somebody drags across, and the few readings inside it.
 *
 * The page is a page of controls, so it refuses selection as a whole — a drag over it is a drag
 * towards a control, and a selection smeared over three rows of labels and their explanations is
 * noise the person then has to clear. What it also holds is a handful of things people take away
 * from it, and those are exempted one at a time rather than as a class.
 *
 * Neither half is visible to the guard in `scripts/tests/selection.test.mjs`: it reads files, and
 * this is one file holding both answers. The classes are what is checked here, the same way the file
 * listing's and the monitor panel's own specs check theirs — a test document lays nothing out, so
 * the absence of a selection is not something an assertion could read back out of the DOM.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  appInfo: vi.fn(async () => ({
    name: "7Shell",
    version: "0.1.0",
    os: "windows",
    arch: "x86_64",
    debugBuild: true,
    protocolCount: 1,
    descriptorSchemaVersion: 1,
  })),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
  webviewSetZoom: vi.fn(),
  keysList: vi.fn(async () => []),
  sessionsList: vi.fn(async () => []),
  groupsList: vi.fn(async () => []),
}));

import SettingsPage from "@/components/settings/SettingsPage.vue";
import { i18n } from "@/i18n";
import { settingsGet } from "@/lib/backend/tauri";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";

const settingsGetMock = vi.mocked(settingsGet);

/** What the backend answers when nothing has been chosen. */
function defaults(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    themeMode: "system",
    themePalette: "default",
    uiScale: 100,
    terminalTheme: "system",
    // Both pages' choices are on the appearance tab, and the fixture is what the store reads: a
    // field left out here is not "unset", it is a control drawn with no value at all.
    cornerStyle: "large",
    homeLayout: "workspace",
    appLayout: "separated",
    tabLayout: "scroll",
    terminalFontSize: 13,
    showTrayIcon: true,
    quitOnClose: false,
    backgroundImage: { displayMode: "fill", transparency: 15, blur: 4 },
    filesLayout: "tree",
    sessionLayout: "split",
    sidebarOpen: true,
    monitorEnabled: true,
    packedTransfer: false,
    ...overrides,
  };
}

let host: HTMLDivElement;
let app: App | undefined;

async function mount(): Promise<void> {
  app = createApp({ render: () => h(SettingsPage) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await usePreferencesStore().load();
  await useAppStore().load();
  await nextTick();
}

/** A category, pressed the way the navigation draws it. */
async function openCategory(name: string): Promise<void> {
  const button = [...host.querySelectorAll("button")].find((candidate) => candidate.textContent?.trim() === name);
  if (!button) {
    throw new Error(`the navigation has no ${name}: ${host.textContent}`);
  }
  button.click();
  await nextTick();
}

/** The elements on the page that carry a class, found by what they are. */
function searchField(): Element {
  const field = host.querySelector('input[type="search"]');
  if (!field) {
    throw new Error("the page draws no search field");
  }
  return field;
}

function selectsText(element: Element | null, what: string): void {
  if (!element) {
    throw new Error(`${what} is not drawn: ${host.textContent}`);
  }
  expect(element.classList.contains("select-text"), `${what} is text`).toBe(true);
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.appendChild(host);
  settingsGetMock.mockReset().mockResolvedValue(defaults() as never);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the settings page", () => {
  it("refuses to be read as a document", async () => {
    await mount();

    const page = host.querySelector("section");
    if (!page) {
      throw new Error("the page is not there");
    }
    expect(page.classList.contains("select-none"), "a drag across the page takes nothing away").toBe(true);
  });

  it("keeps the field somebody types in selectable", async () => {
    await mount();

    // A surface that takes typing is a document while the keyboard is in it, whatever the page
    // around it says — and an inherited `select-none` is exactly what would take that away.
    selectsText(searchField(), "the search field");
  });

  it("keeps an error selectable, because an error is quoted somewhere", async () => {
    settingsGetMock.mockRejectedValue(new Error("the file is not readable"));
    await mount();

    selectsText(host.querySelector("[data-settings-problem]"), "the banner");
  });

  it("keeps the build's own facts selectable", async () => {
    await mount();
    await openCategory("About");

    // A version and a platform go into bug reports; the label beside each is a label.
    const versions = [...host.querySelectorAll("dd")].map((value) => value.textContent?.trim());
    expect(versions, "the About category lists the build's facts").toContain("0.1.0");
    selectsText(host.querySelector("dl"), "the list of facts");
  });

  it("draws the five colours of a hand-picked palette, and the way back to the defaults", async () => {
    settingsGetMock.mockResolvedValue(defaults({ themePalette: "custom" }) as never);
    await mount();

    const pickers = [...host.querySelectorAll<HTMLInputElement>('input[type="color"]')];
    expect(
      // Named by the label beside them rather than by an attribute: a picker is a form control, and
      // what a reader calls it is the word next to it.
      pickers.map((picker) => host.querySelector(`label[for="${picker.id}"]`)?.textContent?.trim()),
      "one picker per colour of the palette, in the order the setting lists them",
    ).toEqual(["Background", "Foreground", "Accent", "Border", "Panels"]);
    // Which half of the palette is being edited is on the page, because a light window showing the
    // dark set's colours would be a form about something nobody can see.
    expect(host.textContent).toContain("Editing: dark");
    expect(host.textContent).toContain("Reset the colours");
  });

  it("draws the tab bar's own choices in one row, each with the line that says what it is for", async () => {
    await mount();
    await openCategory("Appearance");

    for (const title of ["Tab bar position", "Group by", "Sort by", "Maximum tab width"]) {
      expect(host.textContent, `${title} is on the page`).toContain(title);
    }

    // One grid, four columns — and the only grid whose fields draw their summary: at a quarter of the
    // page's width a control with no words under it is a value nobody can tell apart from the next one.
    const row = [...host.querySelectorAll<HTMLElement>("div.grid")].find((grid) =>
      grid.textContent?.includes("Tab bar position"),
    );
    expect(row?.className, "four to a row where this page has room for four").toContain("@4xl:grid-cols-4");
    expect(row?.textContent, "with the line under the control").toContain("Side tab bars scroll vertically");
  });

  it("keeps the keys and hosts this device holds selectable, inside a page that is not", async () => {
    await mount();
    await openCategory("Security");

    // A fingerprint is checked against another machine by eye and a host and port are pasted into a
    // terminal, so both lists keep their text even though the page they sit in does not.
    const lists = [...host.querySelectorAll("div.space-y-2")].filter((candidate) =>
      candidate.classList.contains("select-text"),
    );
    expect(lists.length, "both lists are drawn under the page's refusal").toBe(2);
  });
});

it("finds the shortcuts page by an action's name, or by the key it answers to", async () => {
  await mount();

  const field = searchField() as HTMLInputElement;
  const search = async (query: string): Promise<void> => {
    field.value = query;
    field.dispatchEvent(new Event("input"));
    await nextTick();
  };

  await search("find in the terminal");
  expect(host.textContent, "the page that holds it is drawn, since a key is not a row").toContain(
    "Find in the terminal",
  );
  expect(host.textContent).toContain("Ctrl + F");

  await search("ctrl+f");
  expect(host.textContent, "somebody who knows the key and not its name").toContain("Find in the terminal");

  await search("nothing says this");
  expect(host.textContent).not.toContain("Find in the terminal");
});
