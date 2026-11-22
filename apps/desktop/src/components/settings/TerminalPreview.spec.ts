/**
 * The terminal preview, as the settings beside it make it.
 *
 * The palette read and the contrast arithmetic have their own tests; what is asserted here is that
 * the settings the picture exists for actually reach it — the cursor's shape and blink, the hollow
 * one a terminal shows when it is not focused, bold text choosing bright colours or not, the
 * minimum contrast lifting a line, and a highlight rule drawn the way its mode says.
 *
 * The colours come from a scheme of the block's own, in hex: the contrast arithmetic reads values
 * rather than guessing, and a scheme keeps the whole chain inside the test.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import TerminalPreview from "@/components/settings/TerminalPreview.vue";
import { i18n } from "@/i18n";
import { usePreferencesStore } from "@/stores/preferences";
import type { TerminalHighlightRule } from "@/types/app";

const SCHEME = {
  id: "preview-test",
  name: "Preview test",
  foreground: "#e8e8e8",
  background: "#101010",
  cursor: "#e8e8e8",
  colors: [
    "#101010",
    "#401010",
    "#20c020",
    "#c0a010",
    "#2060c0",
    "#b040b0",
    "#20b0b0",
    "#d0d0d0",
    "#505050",
    "#804040",
    "#40e040",
    "#e0d040",
    "#4080e0",
    "#e060e0",
    "#40d0d0",
    "#f0f0f0",
  ],
};

const ERROR_RULE: TerminalHighlightRule = {
  id: "rule-error",
  pattern: "error",
  color: "#ef4444",
  isRegex: false,
  caseSensitive: false,
};

let host: HTMLDivElement;
let app: App | undefined;

function render(): void {
  app = createApp({ render: () => h(TerminalPreview) }).use(i18n);
  app.mount(host);
}

/** The preview answers once the palette read has run: mount, then let two turns pass. */
async function settled(): Promise<void> {
  await nextTick();
  await nextTick();
  await nextTick();
}

function cursorCells(): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>(".preview-cursor")];
}

/** The first coloured segment of a line — the whole line, unless a rule split it. */
function firstSegment(rowIndex: number): HTMLElement {
  const row = host.querySelectorAll("pre > span")[rowIndex];
  const segment = row?.querySelector("span");
  if (!segment) {
    throw new Error(`the preview has no segment on row ${rowIndex}`);
  }
  return segment;
}

function segmentWithText(rowIndex: number, text: string): HTMLElement {
  const row = host.querySelectorAll("pre > span")[rowIndex];
  const found = [...(row?.querySelectorAll("span") ?? [])].find((entry) => entry.textContent === text);
  if (!found) {
    throw new Error(`the preview has no "${text}" segment on row ${rowIndex}`);
  }
  return found;
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  const preferences = usePreferencesStore();
  preferences.terminalTheme = "dark";
  preferences.addTerminalSchemes([SCHEME]);
  preferences.terminalSchemeSource = "custom";
  preferences.terminalSchemeDark = SCHEME.id;
});

afterEach(() => {
  app?.unmount();
  host.remove();
});

describe("the terminal preview", () => {
  it("draws the cursor in the shape, blink and inactive style the settings name", async () => {
    render();
    await settled();

    const preferences = usePreferencesStore();
    // The default answer: a filled block that blinks, and a hollow one where the terminal is not
    // focused.
    expect(cursorCells()).toHaveLength(2);
    expect(cursorCells()[0]!.className).toContain("blink");
    expect(cursorCells()[1]!.style.border).not.toBe("");

    preferences.terminalCursorBlink = false;
    preferences.terminalCursorStyle = "bar";
    preferences.terminalCursorInactiveStyle = "none";
    await settled();

    expect(cursorCells()).toHaveLength(1);
    expect(cursorCells()[0]!.className).not.toContain("blink");
    expect(cursorCells()[0]!.style.width).toBe("2px");
  });

  it("switches the bold line between the bright colours and the regular ones", async () => {
    render();
    await settled();

    const preferences = usePreferencesStore();
    const bright = firstSegment(0).style.color;
    expect(bright).not.toBe("");

    preferences.terminalBoldBright = false;
    await settled();

    const regular = firstSegment(0).style.color;
    expect(regular).not.toBe("");
    expect(regular).not.toBe(bright);
  });

  it("lifts a low-contrast line when the minimum demands it, and puts it back", async () => {
    render();
    await settled();

    const preferences = usePreferencesStore();
    const raw = firstSegment(4).style.color; // the error line, a dark red on a dark ground
    expect(raw).toBe("#401010");

    preferences.terminalMinContrast = 7;
    await settled();
    const raised = firstSegment(4).style.color;
    expect(raised).not.toBe(raw);

    preferences.terminalMinContrast = 1;
    await settled();
    expect(firstSegment(4).style.color).toBe(raw);
  });

  it("draws a highlight rule the way its mode says", async () => {
    const preferences = usePreferencesStore();
    preferences.terminalHighlightRules = [ERROR_RULE];
    render();
    await settled();

    // Background mode: the word keeps its colour and takes a wash.
    const washed = segmentWithText(4, "error");
    expect(washed.style.background).not.toBe("");
    expect(washed.style.color).toBe("#401010");

    preferences.terminalHighlightMode = "text";
    await settled();
    const repainted = segmentWithText(4, "error");
    expect(repainted.style.background).toBe("");
    expect(repainted.style.color).toBe("#ef4444");
  });

  it("draws a rule's colour as written while the line under it is still lifted", async () => {
    const preferences = usePreferencesStore();
    preferences.terminalHighlightRules = [ERROR_RULE];
    preferences.terminalMinContrast = 7;
    render();
    await settled();

    // The word takes the line's own (lifted) colour — the minimum speaks to the line — and a wash
    // in the rule's colour, which no minimum rewrites.
    const washed = segmentWithText(4, "error");
    const before = host.querySelectorAll("pre > span")[4]!.querySelector("span")!;
    expect(washed.style.color).toBe(before.style.color);
    const wash = washed.style.background;
    const liftedColour = washed.style.color;
    expect(wash).not.toBe("");

    preferences.terminalMinContrast = 1;
    await settled();
    const lowered = segmentWithText(4, "error");
    expect(lowered.style.color).not.toBe(liftedColour);
    expect(lowered.style.background).toBe(wash);
  });
});
