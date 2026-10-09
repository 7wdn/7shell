import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  snippetsOpen: vi.fn(),
  snippetSave: vi.fn(),
  snippetDelete: vi.fn(),
  snippetGroupSave: vi.fn(),
  snippetGroupDelete: vi.fn(),
  commandHistory: vi.fn(),
  commandHistoryClear: vi.fn(),
  commandClassify: vi.fn(),
  commandSend: vi.fn(),
  sessionOpen: vi.fn(),
  terminalWrite: vi.fn(),
}));

import { snippetDelete, snippetGroupDelete, snippetSave, snippetsOpen } from "@/lib/backend/tauri";
import { BUILTIN_COMMANDS } from "@/lib/commands/builtin";
import { fillSlots, slotsIn, useCommandsStore } from "@/stores/commands";
import type { Snippet, SnippetGroup } from "@/types/snippet";

const listMock = vi.mocked(snippetsOpen);
const saveMock = vi.mocked(snippetSave);
const deleteMock = vi.mocked(snippetDelete);
const groupDeleteMock = vi.mocked(snippetGroupDelete);

function snippet(id: string, name: string, order: number, command = "uptime", enabled = true): Snippet {
  return {
    id,
    groupId: "default",
    name,
    prefix: "",
    command,
    appendCr: true,
    enabled,
    order,
    slotHistory: [],
  };
}

function library(commands: Snippet[]): SnippetGroup[] {
  return [{ id: "default", name: "", order: 0, commands }];
}

/** The shipped list as it comes back from storage: same order, same words, nothing switched off. */
function shippedRows(): Snippet[] {
  return BUILTIN_COMMANDS.map((line, index) => ({
    ...snippet(`snippet-${index}`, line.name, index + 1, line.command),
    prefix: line.prefix,
  }));
}

/**
 * One of the lines this build ships, as the library would hold it.
 *
 * A library that holds one of these is left alone by the seeding, which is what the tests that are
 * not about the seeding want: their fixtures are about moving and switching, and twenty writes
 * arriving behind their backs would be twenty writes their assertions have to look past.
 */
function shipped(): Snippet {
  const line = BUILTIN_COMMANDS[0] ?? { name: "", prefix: "", command: "" };
  return { ...snippet("shipped", line.name, 0, line.command), prefix: line.prefix };
}

describe("the command library", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("reads the slots a line uses, in order and without repeats", () => {
    expect(slotsIn("systemctl restart #1")).toEqual([1]);
    expect(slotsIn("cp #2 #1 #2")).toEqual([1, 2]);
    expect(slotsIn("uptime")).toEqual([]);
    // `#0` and `#9` are not slots this build knows about, and a line with them sends as it stands.
    expect(slotsIn("echo #0 #9")).toEqual([]);
  });

  it("fills what has a value and leaves the rest visibly unfilled", () => {
    expect(fillSlots("cp #1 #2", ["/a/b", "/c"])).toBe("cp /a/b /c");
    expect(fillSlots("cp #1 #2", ["/a/b"])).toBe("cp /a/b #2");
    expect(fillSlots("cp #1", ["  "])).toBe("cp #1");
  });

  it("moves a command past its neighbour by swapping the order they are stored with", async () => {
    const commands = useCommandsStore();
    listMock.mockResolvedValue(library([shipped(), snippet("a", "first", 1), snippet("b", "second", 2)]));
    saveMock.mockResolvedValue(snippet("b", "second", 1));
    await commands.load();

    await commands.move("b", -1);

    // Two writes, each carrying the other's order: the list is not renumbered, it is swapped.
    expect(saveMock).toHaveBeenNthCalledWith(1, expect.objectContaining({ id: "a", order: 2 }));
    expect(saveMock).toHaveBeenNthCalledWith(2, expect.objectContaining({ id: "b", order: 1 }));
  });

  it("does not move what is already at the end it is being pushed towards", async () => {
    const commands = useCommandsStore();
    listMock.mockResolvedValue(library([snippet("a", "first", 1), shipped(), snippet("b", "second", 2)]));
    await commands.load();

    await commands.move("a", -1);
    await commands.move("b", 1);

    expect(saveMock).not.toHaveBeenCalled();
  });

  it("fills a library that holds none of the shipped lines with the commands this build ships", async () => {
    const commands = useCommandsStore();
    listMock
      .mockResolvedValueOnce(library([snippet("mine", "重启 nginx", 1)]))
      .mockResolvedValueOnce(library([snippet("mine", "重启 nginx", 1), snippet("shipped", "df -h", 2, "df -h")]));
    saveMock.mockResolvedValue(snippet("shipped", "df -h", 2, "df -h"));

    await commands.load();

    // Having lines of one's own does not mean having the shipped list: somebody who has been using
    // the application for a while would otherwise never see it at all.
    expect(saveMock).toHaveBeenCalledTimes(BUILTIN_COMMANDS.length);
    // Written as ordinary commands, into the group a fresh library has: nothing marks them as ours,
    // which is what lets them be edited and deleted like anything else.
    expect(saveMock).toHaveBeenCalledWith(
      expect.objectContaining({ id: "", groupId: "default", prefix: "dfh", command: "df -h", enabled: true }),
    );
    expect(listMock, "the list is read again, so what is on screen is what was written").toHaveBeenCalledTimes(2);
    expect(commands.groups[0]?.commands).toHaveLength(2);
  });

  it("leaves a library that already holds one of the shipped lines alone", async () => {
    const commands = useCommandsStore();
    // The name is what says the list is already there, so one line somebody kept — with their own
    // words for it, or with the shipped name — is enough to stop the rest being written in.
    listMock.mockResolvedValue(library([snippet("mine", "重启 nginx", 1), shipped()]));

    await commands.load();

    expect(saveMock, "a list that is put back on every read is a list nobody can prune").not.toHaveBeenCalled();
  });

  it("puts the library back to the shipped list, dropping everything else", async () => {
    const commands = useCommandsStore();
    const mine = { ...snippet("mine", "重启 nginx", 21), groupId: "snippet-group-1" };
    // One of the shipped lines is already in the library, so no read in this test seeds anything:
    // what the restore does about the shipped list is what is being asked.
    const kept = { ...snippet("kept", "df -h", 1), prefix: "dfh" };
    listMock
      .mockResolvedValueOnce([
        { id: "default", name: "", order: 0, commands: [kept] },
        { id: "snippet-group-1", name: "maintenance", order: 1, commands: [mine] },
      ])
      .mockResolvedValueOnce([{ id: "default", name: "", order: 0, commands: [] }])
      .mockResolvedValue(library(shippedRows()));
    deleteMock.mockResolvedValue();
    groupDeleteMock.mockResolvedValue();

    await commands.load();
    expect(commands.needsRestore, "somebody has lines of their own").toBe(true);

    await commands.restoreDefaults();

    // Everything goes, the groups somebody made as well, and the shipped list comes back through the
    // ordinary first-install rule: there is one answer to what a fresh library holds.
    expect(deleteMock).toHaveBeenCalledWith("kept");
    expect(deleteMock).toHaveBeenCalledWith("mine");
    expect(groupDeleteMock).toHaveBeenCalledWith("snippet-group-1");
    expect(saveMock, "and the shipped lines are written again").toHaveBeenCalled();
    expect(commands.needsRestore, "and there is nothing left to restore").toBe(false);
  });

  it("has nothing to restore once the library is the shipped list", async () => {
    const commands = useCommandsStore();
    listMock.mockResolvedValue(library(shippedRows()));

    await commands.load();

    expect(commands.needsRestore, "a button that is always available on an untouched library does nothing").toBe(false);
  });

  it("switches a command off by writing it, not by keeping a flag beside it", async () => {
    const commands = useCommandsStore();
    const stored = shipped();
    listMock.mockResolvedValue(library([stored]));
    saveMock.mockResolvedValue({ ...stored, enabled: false });

    await commands.load();
    await commands.setEnabled(stored, false);

    expect(saveMock).toHaveBeenCalledWith({ ...stored, enabled: false });
  });

  // Sending, and therefore remembering the values a slot was filled with, needs a terminal that
  // is open — which this suite has no way to have. The storage side of that is covered by the
  // vault's own tests (`Snippet::remember`), and the end-to-end case covers the rest: a command
  // with a `#1` reaches a real shell, substituted.
});
