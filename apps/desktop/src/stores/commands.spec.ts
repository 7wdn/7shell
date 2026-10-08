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

import { snippetsOpen, snippetSave } from "@/lib/backend/tauri";
import { fillSlots, slotsIn, useCommandsStore } from "@/stores/commands";
import type { Snippet, SnippetGroup } from "@/types/snippet";

const listMock = vi.mocked(snippetsOpen);
const saveMock = vi.mocked(snippetSave);

function snippet(id: string, name: string, order: number, command = "uptime"): Snippet {
  return {
    id,
    groupId: "default",
    name,
    prefix: "",
    command,
    appendCr: true,
    order,
    slotHistory: [],
  };
}

function library(commands: Snippet[]): SnippetGroup[] {
  return [{ id: "default", name: "", order: 0, commands }];
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
    listMock.mockResolvedValue(library([snippet("a", "first", 1), snippet("b", "second", 2)]));
    saveMock.mockResolvedValue(snippet("b", "second", 1));
    await commands.load();

    await commands.move("b", -1);

    // Two writes, each carrying the other's order: the list is not renumbered, it is swapped.
    expect(saveMock).toHaveBeenNthCalledWith(1, expect.objectContaining({ id: "a", order: 2 }));
    expect(saveMock).toHaveBeenNthCalledWith(2, expect.objectContaining({ id: "b", order: 1 }));
  });

  it("does not move what is already at the end it is being pushed towards", async () => {
    const commands = useCommandsStore();
    listMock.mockResolvedValue(library([snippet("a", "first", 1), snippet("b", "second", 2)]));
    await commands.load();

    await commands.move("a", -1);
    await commands.move("b", 1);

    expect(saveMock).not.toHaveBeenCalled();
  });

  // Sending, and therefore remembering the values a slot was filled with, needs a terminal that
  // is open — which this suite has no way to have. The storage side of that is covered by the
  // vault's own tests (`Snippet::remember`), and the end-to-end case covers the rest: a command
  // with a `#1` reaches a real shell, substituted.
});
