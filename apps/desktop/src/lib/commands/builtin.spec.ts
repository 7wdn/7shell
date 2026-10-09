/**
 * What "the library is what this build ships" means, and the words that come out of the table.
 */
import { describe, expect, it } from "vitest";

import { BUILTIN_COMMANDS, libraryIsShipped } from "@/lib/commands/builtin";
import type { Snippet, SnippetGroup } from "@/types/snippet";

function shippedLibrary(): Snippet[] {
  return BUILTIN_COMMANDS.map((builtin, index) => ({
    id: `snippet-${index}`,
    groupId: "default",
    name: builtin.name,
    prefix: builtin.prefix,
    command: builtin.command,
    appendCr: true,
    enabled: true,
    order: index + 1,
    slotHistory: [],
  }));
}

function groups(commands: Snippet[], id = "default"): SnippetGroup[] {
  return [{ id, name: "", order: 0, commands }];
}

describe("a library nobody has touched", () => {
  it("is the shipped list, in the order it is shipped in", () => {
    expect(libraryIsShipped(groups(shippedLibrary()))).toBe(true);
  });

  it("is not, once one line says something else", () => {
    const commands = shippedLibrary();
    const first = commands[0] as Snippet;
    const edited: Snippet[] = [{ ...first, command: "ps aux | grep -v grep #1" }, ...commands.slice(1)];

    expect(libraryIsShipped(groups(edited)), "an edited line is a change to restore").toBe(false);
  });

  it("is not, once a line is switched off", () => {
    const commands = shippedLibrary();
    const first = commands[0] as Snippet;
    const off: Snippet[] = [{ ...first, enabled: false }, ...commands.slice(1)];

    expect(libraryIsShipped(groups(off))).toBe(false);
  });

  it("is not, once one of them is gone", () => {
    expect(libraryIsShipped(groups(shippedLibrary().slice(1)))).toBe(false);
  });

  it("is not, once there is a command somebody wrote", () => {
    const commands = [
      ...shippedLibrary(),
      ...shippedLibrary()
        .slice(0, 1)
        .map((entry) => ({ ...entry, name: "mine" })),
    ];

    expect(libraryIsShipped(groups(commands))).toBe(false);
  });

  it("is not, once there is a group somebody made", () => {
    const library = [...groups(shippedLibrary()), ...groups([], "snippet-group-1")];

    expect(libraryIsShipped(library)).toBe(false);
  });
});
