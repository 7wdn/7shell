import { describe, expect, it } from "vitest";

import { parseBinding } from "@/lib/shortcuts/binding";
import { bindingFor, resolveBindings, SHORTCUT_DEFINITIONS } from "@/lib/shortcuts/registry";

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
