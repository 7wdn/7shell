import { describe, expect, it } from "vitest";

import { isMacPlatform, matchesShortcut, parseBinding, type ShortcutEventLike } from "@/lib/shortcuts/binding";

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
