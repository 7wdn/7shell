/**
 * The import dialog's own answers: what it does with pasted text and with a picked file, and that
 * a name on a row is editable right up to the moment the import is taken.
 *
 * The reader's dialects are tested beside the reader; what is asserted here is the flow around
 * them — a rename survives the import, a file's name names its scheme, an unnamed paste names
 * itself, and a failed read is said out loud rather than swallowed.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend/api", () => ({
  backend: {
    settings: {
      pickSchemeImport: vi.fn(),
    },
  },
}));

import TerminalSchemeImportDialog from "@/components/settings/TerminalSchemeImportDialog.vue";
import { i18n } from "@/i18n";
import { backend } from "@/lib/backend/api";
import { normalizeShellError } from "@/lib/backend/errors";
import type { ImportedTerminalScheme } from "@/lib/terminal/importScheme";

const pickMock = vi.mocked(backend.settings.pickSchemeImport);

const COLORS = Array.from({ length: 16 }, (_, index) => `#${(index + 1).toString(16).padStart(2, "0")}2233`);

/** One Windows Terminal scheme, carrying its own name — the shape that names itself. */
function namedJson(): string {
  const names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
  return JSON.stringify({
    name: "Catalogue name",
    foreground: "#ffffff",
    background: "#101010",
    cursorColor: "#ffffff",
    ...Object.fromEntries(names.map((key, index) => [key, COLORS[index]])),
    ...Object.fromEntries(
      names.map((key, index) => [`bright${key[0]!.toUpperCase()}${key.slice(1)}`, COLORS[index + 8]]),
    ),
  });
}

/** The same sixteen colours with no name anywhere — the shape that needs one made up. */
function unnamedJson(): string {
  return JSON.stringify({
    foreground: "#ffffff",
    background: "#101010",
    colors: COLORS,
  });
}

let host: HTMLDivElement | null = null;
let app: App | null = null;
let taken: ImportedTerminalScheme[][] = [];

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  taken = [];
  app = createApp({
    render: () =>
      h(TerminalSchemeImportDialog, {
        open: true,
        onImport: (schemes: readonly ImportedTerminalScheme[]) => {
          taken.push([...schemes]);
        },
        "onUpdate:open": () => undefined,
      }),
  });
  app.use(i18n);
  app.mount(host);
  await nextTick();
  await nextTick();
}

function label(key: string, params?: Record<string, unknown>): string {
  return i18n.global.t(key, params ?? {});
}

function button(text: string): HTMLButtonElement {
  const found = [...document.querySelectorAll("button")].find((entry) => entry.textContent?.trim() === text);
  expect(found, `no button labelled "${text}" is on screen`).toBeDefined();
  return found!;
}

async function paste(text: string): Promise<void> {
  const area = document.querySelector("textarea");
  expect(area, "the paste box is not on screen").toBeDefined();
  area!.value = text;
  area!.dispatchEvent(new Event("input"));
  await nextTick();
  button(label("settings.schemes.import.read")).click();
  await nextTick();
  await nextTick();
}

function nameInputs(): HTMLInputElement[] {
  return [...document.querySelectorAll<HTMLInputElement>("input")];
}

afterEach(() => {
  app?.unmount();
  host?.remove();
  host = null;
  app = null;
  vi.clearAllMocks();
});

describe("the scheme import dialog", () => {
  beforeEach(() => {
    pickMock.mockReset();
  });

  it("renames a recognised scheme before the import is taken", async () => {
    await mount();
    await paste(namedJson());

    const rows = nameInputs();
    expect(rows).toHaveLength(1);
    expect(rows[0]!.value).toBe("Catalogue name");

    rows[0]!.value = "Mine instead";
    rows[0]!.dispatchEvent(new Event("input"));
    await nextTick();

    button(label("settings.schemes.import.commit", { count: 1 })).click();
    await nextTick();

    expect(taken).toHaveLength(1);
    expect(taken[0]).toHaveLength(1);
    expect(taken[0]![0]!.name).toBe("Mine instead");
    expect(taken[0]![0]!.colors).toEqual(COLORS);
  });

  it("names an unnamed paste after the fallback, and a cleared name falls back again", async () => {
    await mount();
    await paste(unnamedJson());

    const fallback = label("settings.schemes.import.fallbackName");
    expect(nameInputs()[0]!.value).toBe(fallback);

    nameInputs()[0]!.value = "   ";
    nameInputs()[0]!.dispatchEvent(new Event("input"));
    await nextTick();
    button(label("settings.schemes.import.commit", { count: 1 })).click();
    await nextTick();

    expect(taken[0]![0]!.name).toBe(fallback);
  });

  it("reads a picked file, keeps its text, and names the scheme after the file", async () => {
    await mount();
    pickMock.mockResolvedValue({ name: "3024 Night", contents: unnamedJson() });

    button(label("settings.schemes.import.pickFile")).click();
    await vi.waitFor(() => expect(nameInputs()).toHaveLength(1));

    expect(pickMock).toHaveBeenCalledWith(label("settings.schemes.import.pickTitle"));
    expect(nameInputs()[0]!.value).toBe("3024 Night");

    // The file's text was kept: going back shows it in the paste box, ready to be edited.
    button(label("settings.schemes.import.back")).click();
    await nextTick();
    expect(document.querySelector("textarea")?.value).toBe(unnamedJson());
  });

  it("says so when the picked file cannot be read", async () => {
    await mount();
    pickMock.mockRejectedValue(
      normalizeShellError(
        { code: "SCHEME-IMPORT-UNREADABLE", messageKey: "errors.scheme.importUnreadable", origin: { subsystem: "fs" } },
        "scheme_import_pick",
      ),
    );

    button(label("settings.schemes.import.pickFile")).click();
    await vi.waitFor(() => expect(document.body.textContent).toContain(label("errors.scheme.importUnreadable")));
    // Nothing was recognised, so the naming stage never opened and nothing can be taken.
    expect(nameInputs()).toHaveLength(0);
    expect(taken).toHaveLength(0);
  });

  it("refuses text that is not a scheme, and lets the paste go on being edited", async () => {
    await mount();
    await paste("hello");

    expect(document.body.textContent).toContain(label("settings.schemes.import.empty"));
    expect(nameInputs()).toHaveLength(0);
    expect(taken).toHaveLength(0);
  });

  it("drops one row of many, and imports the rest", async () => {
    await mount();
    await paste(JSON.stringify([JSON.parse(namedJson()), JSON.parse(unnamedJson())]));

    expect(nameInputs()).toHaveLength(2);
    // The drop buttons carry an icon and an aria-label rather than any text; the second one
    // drops the unnamed row, and the named one is what the import then carries.
    const drop = [
      ...document.querySelectorAll<HTMLButtonElement>(
        `button[aria-label="${label("settings.schemes.import.dropRow")}"]`,
      ),
    ];
    expect(drop).toHaveLength(2);
    drop[1]!.click();
    await nextTick();

    expect(nameInputs()).toHaveLength(1);
    button(label("settings.schemes.import.commit", { count: 1 })).click();
    await nextTick();

    expect(taken[0]).toHaveLength(1);
    expect(taken[0]![0]!.name).toBe("Catalogue name");
  });
});
