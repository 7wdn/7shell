import { describe, expect, it } from "vitest";

import {
  eventToBinding,
  isMacPlatform,
  matchesShortcut,
  normalizeBinding,
  parseBinding,
  type ShortcutEventLike,
} from "@/lib/shortcuts/binding";

/** A key press with the modifiers nobody asked for left up. */
function press(key: string, held: Partial<ShortcutEventLike> = {}): ShortcutEventLike {
  return { key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...held };
}

describe("reading a binding", () => {
  it("keeps the modifiers in a fixed order however they were written", () => {
    expect(parseBinding("shift+mod+k")).toEqual({ modifiers: ["Mod", "Shift"], key: "k" });
  });

  it("answers null for a binding nothing can match", () => {
    expect(parseBinding("")).toBeNull();
    expect(parseBinding("Mod+")).toBeNull();
    expect(parseBinding("Hyper+V")).toBeNull();
  });
});

describe("matching an event", () => {
  it("reads Mod as Ctrl on Windows and as Command on macOS", () => {
    expect(matchesShortcut(press("v", { ctrlKey: true }), "Mod+V", "windows x86_64")).toBe(true);
    expect(matchesShortcut(press("v", { metaKey: true }), "Mod+V", "macos aarch64")).toBe(true);
  });

  it("keeps Ctrl and Command apart on macOS, so a binding for one is not the other", () => {
    expect(matchesShortcut(press("v", { ctrlKey: true }), "Mod+V", "macos aarch64")).toBe(false);
    expect(matchesShortcut(press("v", { metaKey: true }), "Mod+V", "windows x86_64")).toBe(false);
  });

  it("requires a modifier the binding does not name to be up, so Ctrl+Shift+F is not Ctrl+F", () => {
    expect(matchesShortcut(press("f", { ctrlKey: true, shiftKey: true }), "Mod+F", "linux x86_64")).toBe(false);
  });

  it("matches a character by the physical key when the layout or Alt rewrote it", () => {
    // macOS turns Alt+/ into ÷ : the character is gone, the key is not.
    expect(matchesShortcut(press("÷", { altKey: true, code: "Slash" }), "Alt+/", "macos aarch64")).toBe(true);
    expect(matchesShortcut(press("÷", { altKey: true }), "Alt+/", "macos aarch64")).toBe(false);
  });

  it("compares letters without case, because Shift is what makes a capital", () => {
    expect(matchesShortcut(press("V", { ctrlKey: true }), "Mod+V", "windows x86_64")).toBe(true);
  });

  it("matches nothing for a binding nothing can parse", () => {
    expect(matchesShortcut(press("v", { ctrlKey: true }), "", "windows x86_64")).toBe(false);
    expect(matchesShortcut(press("v", { ctrlKey: true }), "Hyper+V", "windows x86_64")).toBe(false);
  });
});

describe("reading the platform", () => {
  it("knows macOS from the string the host reports", () => {
    expect(isMacPlatform("macos aarch64")).toBe(true);
    expect(isMacPlatform("windows x86_64")).toBe(false);
    expect(isMacPlatform("")).toBe(false);
  });
});

describe("writing a binding down", () => {
  it("keeps the table's order and the case a single character is written in", () => {
    expect(normalizeBinding("shift+mod+k")).toBe("Mod+Shift+K");
    expect(normalizeBinding("alt+/")).toBe("Alt+/");
    expect(normalizeBinding("Tab")).toBe("Tab");
    expect(normalizeBinding("Hyper+V")).toBeNull();
  });

  it("reads a press as the binding this application stores", () => {
    expect(eventToBinding(press("v", { ctrlKey: true }), "windows x86_64")).toBe("Mod+V");
    expect(eventToBinding(press("k", { ctrlKey: true, shiftKey: true }), "linux x86_64")).toBe("Mod+Shift+K");
    expect(eventToBinding(press("ArrowUp", { altKey: true }), "linux x86_64")).toBe("Alt+ArrowUp");
  });

  it("answers nothing for a press that names no key", () => {
    expect(eventToBinding(press("Shift", { shiftKey: true }), "windows x86_64")).toBeNull();
    expect(eventToBinding(press("Control", { ctrlKey: true }), "windows x86_64")).toBeNull();
  });

  it("writes the platform's own modifier as Mod, so a binding travels", () => {
    // Recorded on a Mac, where Command is the modifier; read on Windows, where Ctrl is. One stored
    // string, one key for the action — which is why the storage spelling is not the platform's.
    const recorded = eventToBinding(press("g", { metaKey: true }), "macos aarch64");
    expect(recorded).toBe("Mod+G");
    expect(matchesShortcut(press("g", { ctrlKey: true }), recorded ?? "", "windows x86_64")).toBe(true);
  });

  it("keeps Ctrl and Command apart where the platform does", () => {
    expect(eventToBinding(press("g", { ctrlKey: true }), "macos aarch64")).toBe("Ctrl+G");
    expect(eventToBinding(press("g", { metaKey: true }), "windows x86_64")).toBe("Meta+G");
  });
});
