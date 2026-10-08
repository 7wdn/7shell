import { describe, expect, it } from "vitest";

import { parseBinding } from "@/lib/shortcuts/binding";
import { bindingFor, resolveBindings, SHORTCUT_DEFINITIONS, SHORTCUT_SCOPE_ORDER } from "@/lib/shortcuts/registry";

import enSettings from "@/i18n/locales/en/settings.json";
import zhSettings from "@/i18n/locales/zh-CN/settings.json";

describe("the table of reserved keys", () => {
  it("names each action once", () => {
    const ids = SHORTCUT_DEFINITIONS.map((definition) => definition.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("holds bindings that can be read, so none of them is dead on arrival", () => {
    for (const definition of SHORTCUT_DEFINITIONS) {
      expect(parseBinding(definition.defaultBinding), definition.id).not.toBeNull();
    }
  });

  it("gives two actions in the same surface different keys", () => {
    const taken = new Map<string, string>();
    for (const definition of SHORTCUT_DEFINITIONS) {
      const key = `${definition.scope}:${definition.defaultBinding.toLowerCase()}`;
      const other = taken.get(key);
      expect(other, `${definition.id} and ${other} both answer to ${definition.defaultBinding}`).toBeUndefined();
      taken.set(key, definition.id);
    }
  });
});

describe("reading a binding out of the table", () => {
  it("answers the default while nobody has replaced anything", () => {
    expect(bindingFor("terminal.find")).toBe("Mod+F");
    expect(bindingFor("commands.completionAccept")).toBe("Tab");
    expect(bindingFor("commands.completionTrigger")).toBe("Alt+/");
    expect(Object.keys(resolveBindings())).toHaveLength(SHORTCUT_DEFINITIONS.length);
  });

  it("leaves Ctrl+Space to the input method, which owns it on Windows", () => {
    expect(SHORTCUT_DEFINITIONS.map((definition) => definition.defaultBinding.toLowerCase())).not.toContain(
      "ctrl+space",
    );
  });

  it("lets an override stand in for the default", () => {
    expect(bindingFor("terminal.find", { "terminal.find": "Mod+G" })).toBe("Mod+G");
  });

  it("treats a cleared override as unbound rather than as the default again", () => {
    expect(bindingFor("terminal.find", { "terminal.find": "" })).toBe("");
  });
});

/** What a locale file holds under a dotted key, or nothing when it is not there. */
function named(source: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], source);
}

describe("what each action is called", () => {
  it("has a name in both languages, so the settings page has something to list", () => {
    for (const definition of SHORTCUT_DEFINITIONS) {
      expect(named(enSettings, definition.labelKey), definition.id + " in English").toEqual(expect.any(String));
      expect(named(zhSettings, definition.labelKey), definition.id + " in Chinese").toEqual(expect.any(String));
    }
  });

  it("names the surfaces it lists them under, in both languages", () => {
    for (const scope of SHORTCUT_SCOPE_ORDER) {
      const key = "settings.shortcuts.scope." + scope;
      expect(named(enSettings, key), scope + " in English").toEqual(expect.any(String));
      expect(named(zhSettings, key), scope + " in Chinese").toEqual(expect.any(String));
    }
  });
});
