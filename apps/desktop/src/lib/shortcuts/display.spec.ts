import { describe, expect, it } from "vitest";

import { formatShortcutDisplay } from "@/lib/shortcuts/display";

const WINDOWS = "windows x86_64";
const MAC = "macos aarch64";

describe("spelling a binding out", () => {
  it("names the modifiers on Windows and joins them the way a menu does", () => {
    expect(formatShortcutDisplay("Mod+V", WINDOWS)).toBe("Ctrl + V");
    expect(formatShortcutDisplay("Shift+Mod+K", WINDOWS)).toBe("Ctrl + Shift + K");
    expect(formatShortcutDisplay("Alt+/", WINDOWS)).toBe("Alt + /");
  });

  it("uses the system glyphs on macOS", () => {
    expect(formatShortcutDisplay("Mod+V", MAC)).toBe("⌘ V");
    expect(formatShortcutDisplay("Alt+/", MAC)).toBe("⌥ /");
  });

  it("spells named keys the way each platform writes them", () => {
    expect(formatShortcutDisplay("Enter", WINDOWS)).toBe("Enter");
    expect(formatShortcutDisplay("Enter", MAC)).toBe("↩");
    expect(formatShortcutDisplay("Tab", WINDOWS)).toBe("Tab");
    expect(formatShortcutDisplay("Tab", MAC)).toBe("⇥");
  });

  it("shows nothing for a binding that was cleared", () => {
    expect(formatShortcutDisplay("", WINDOWS)).toBe("");
  });

  it("prints a binding it cannot parse rather than hiding it", () => {
    expect(formatShortcutDisplay("Hyper+V", WINDOWS)).toBe("Hyper+V");
  });
});
