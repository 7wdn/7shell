/**
 * The command panel's groups, as the sidebar's groups behave: a name is typed in the row it belongs
 * to, and a group that is being made appears first and is named afterwards.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

import CommandsPanel from "@/components/commands/CommandsPanel.vue";
import { i18n } from "@/i18n";
import { commandHistory, snippetGroupSave, snippetsOpen } from "@/lib/backend/tauri";
import { BUILTIN_COMMANDS } from "@/lib/commands/builtin";
import type { Snippet, SnippetGroup } from "@/types/snippet";

const listMock = vi.mocked(snippetsOpen);
const historyMock = vi.mocked(commandHistory);
const groupSaveMock = vi.mocked(snippetGroupSave);

let app: App | undefined;
let host: HTMLDivElement;

const shipped: Snippet = {
  id: "shipped",
  groupId: "default",
  name: BUILTIN_COMMANDS[0]?.name ?? "uptime",
  prefix: "",
  command: "uptime",
  appendCr: true,
  enabled: true,
  order: 0,
  slotHistory: [],
};

const defaultGroup: SnippetGroup = { id: "default", name: "", order: 0, commands: [shipped] };
const ops: SnippetGroup = { id: "g1", name: "Ops", order: 1, commands: [] };

/** Mounts the panel over a library, with what later reads answer with. */
async function mount(groups: readonly SnippetGroup[], then: readonly SnippetGroup[] = groups): Promise<void> {
  listMock.mockReset();
  listMock.mockResolvedValueOnce([...groups]);
  listMock.mockResolvedValue([...then]);
  historyMock.mockResolvedValue([]);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(CommandsPanel) }).use(i18n);
  app.mount(host);
  await vi.waitFor(() => expect(host.textContent).toContain("Ops"));
}

function groupRow(name: string): HTMLElement {
  const row = [...host.querySelectorAll<HTMLElement>("ul li > div")].find((candidate) =>
    candidate.textContent?.includes(name),
  );
  if (!row) {
    throw new Error(`no group row says ${name}: ${host.textContent ?? ""}`);
  }
  return row;
}

function nameField(): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>('input[aria-label="Group name"]');
}

function press(button: HTMLElement, key: string): void {
  button.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

beforeEach(() => {
  setActivePinia(createPinia());
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("the command panel's groups", () => {
  it("renames in the row, through the group's own menu button", async () => {
    await mount([defaultGroup, ops]);

    groupRow("Ops").querySelector<HTMLButtonElement>('button[aria-label="Rename"]')?.click();
    await nextTick();

    const field = nameField();
    expect(field, "the row itself is the field").not.toBeNull();
    expect(field?.value).toBe("Ops");

    if (field) {
      field.value = "Prod";
      field.dispatchEvent(new Event("input"));
      press(field, "Enter");
    }

    await vi.waitFor(() =>
      expect(groupSaveMock).toHaveBeenCalledWith({ id: "g1", name: "Prod", order: 1, commands: [] }),
    );
  });

  it("makes the group first and names it in the row it already owns", async () => {
    const created: SnippetGroup = { id: "g2", name: "New group", order: 0, commands: [] };
    groupSaveMock.mockResolvedValue(created);
    await mount([defaultGroup, ops], [defaultGroup, ops, created]);

    host.querySelector<HTMLButtonElement>('button[aria-label="New group"]')?.click();
    await vi.waitFor(() => expect(nameField()).not.toBeNull());

    const field = nameField();
    expect(field?.value).toBe("New group");
    expect(document.activeElement).toBe(field);

    if (field) {
      field.value = "Deploy";
      field.dispatchEvent(new Event("input"));
      press(field, "Enter");
    }

    await vi.waitFor(() =>
      expect(groupSaveMock).toHaveBeenLastCalledWith({ id: "g2", name: "Deploy", order: 0, commands: [] }),
    );
  });

  it("leaves the field when Escape is pressed", async () => {
    await mount([defaultGroup, ops]);

    groupRow("Ops").querySelector<HTMLButtonElement>('button[aria-label="Rename"]')?.click();
    await nextTick();
    const field = nameField();
    if (!field) {
      throw new Error("the row is not being renamed");
    }

    press(field, "Escape");
    await nextTick();

    expect(nameField()).toBeNull();
    expect(groupSaveMock).not.toHaveBeenCalled();
  });
});
