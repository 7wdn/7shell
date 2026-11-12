/**
 * The server-content picker: what it loads, what it draws after a read, and what the two buttons
 * hand on.
 *
 * The store and the host are exercised elsewhere; what is asserted here is what this dialog would
 * get wrong on its own — a catalog that never arrives looking like an empty one, a preview that
 * shows a summary instead of the returned text, an Attach offered for a blob, a prompt that inserts
 * something other than what was previewed.
 */
import { createPinia, setActivePinia, type Pinia } from "pinia";
import { type App, createApp, h } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));

vi.mock("@/stores/terminals", () => ({
  useTerminalsStore: () => ({ activeTerminalHandle: () => null, activeTab: null }),
}));

vi.mock("@/lib/terminal/buffers", () => ({ terminalTail: vi.fn(() => "") }));

vi.mock("@/lib/backend/tauri", () => ({
  mcpContentCatalog: vi.fn(),
  mcpResourceRead: vi.fn(),
  mcpPromptGet: vi.fn(),
}));

import ServerContentDialog from "@/components/assistant/ServerContentDialog.vue";
import { i18n } from "@/i18n";
import { mcpContentCatalog, mcpPromptGet, mcpResourceRead } from "@/lib/backend/tauri";
import { useAssistantStore } from "@/stores/assistant";
import type { McpPromptText, McpResourceRead, McpServerContent } from "@/types/mcp";

const catalogMock = vi.mocked(mcpContentCatalog);
const readMock = vi.mocked(mcpResourceRead);
const promptMock = vi.mocked(mcpPromptGet);

let app: App | undefined;
let pinia: Pinia;

const CATALOG: McpServerContent[] = [
  {
    serverId: "files",
    name: "Files",
    resources: [
      {
        uri: "file:///welcome.txt",
        name: "welcome.txt",
        title: "Welcome",
        mimeType: "text/plain",
        description: "A greeting",
      },
      { uri: "file:///logo.png", name: "logo.png", mimeType: "image/png" },
    ],
    prompts: [
      {
        name: "summarize",
        title: "Summarize",
        description: "Ask for a summary",
        arguments: [
          { name: "topic", required: true },
          { name: "tone", required: false },
        ],
      },
    ],
  },
];

function read(constructs: Partial<McpResourceRead> = {}): McpResourceRead {
  return {
    serverId: "files",
    uri: "file:///welcome.txt",
    contents: [
      { uri: "file:///welcome.txt", mimeType: "text/plain", text: "Hello from the fixture.", truncated: false },
    ],
    truncated: false,
    ...constructs,
  };
}

function rendered(): McpPromptText {
  return {
    serverId: "files",
    name: "summarize",
    messages: [
      { role: "user", text: "Summarize networks" },
      { role: "assistant", text: "Noted." },
    ],
    truncated: false,
  };
}

/** What the dialog asked the panel to insert. */
let inserted: string[] = [];

function mount(): void {
  pinia = createPinia();
  setActivePinia(pinia);
  inserted = [];
  app = createApp({
    render: () =>
      h(ServerContentDialog, {
        open: true,
        onInsert: (text: string) => inserted.push(text),
      }),
  })
    .use(pinia)
    .use(i18n);
  app.mount(document.createElement("div"));
}

/** One turn of the event loop, which is what reka-ui's dismiss layer and teleport need. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function dialog(): HTMLElement {
  const element = document.querySelector<HTMLElement>("[data-mcp-content-dialog]");
  if (!element) {
    throw new Error("the server-content dialog was not drawn");
  }
  return element;
}

function find<T extends HTMLElement>(selector: string): T {
  const element = dialog().querySelector<T>(selector);
  if (!element) {
    throw new Error(`the dialog has no ${selector}`);
  }
  return element;
}

function maybe<T extends HTMLElement>(selector: string): T | null {
  return dialog().querySelector<T>(selector);
}

function click(selector: string): void {
  find<HTMLButtonElement>(selector).click();
}

/** Types into a field the way a person does, so the draft is what changes. */
async function type(selector: string, value: string): Promise<void> {
  const element = find<HTMLInputElement>(selector);
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await settle();
}

beforeEach(() => {
  vi.clearAllMocks();
  catalogMock.mockResolvedValue(CATALOG);
  readMock.mockResolvedValue(read());
  promptMock.mockResolvedValue(rendered());
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  for (const node of document.querySelectorAll("[data-mcp-content-dialog]")) {
    node.remove();
  }
});

describe("what the catalog says", () => {
  it("groups the resources a server offers, with its name, URI and type", async () => {
    mount();
    await vi.waitFor(() => expect(find('[data-mcp-resource="file:///welcome.txt"]')).toBeTruthy());

    const row = find('[data-mcp-resource="file:///welcome.txt"]');
    expect(row.getAttribute("data-mcp-resource-server")).toBe("files");
    expect(row.textContent).toContain("Welcome");
    expect(row.textContent).toContain("file:///welcome.txt");
    expect(row.textContent).toContain("text/plain");
    expect(dialog().textContent).toContain("Files");
    // The second half is a tab away, so a resource listing does not imply prompts exist.
    expect(maybe("[data-mcp-prompt]")).toBeNull();
  });

  it("says plainly when nothing is offered, instead of looking like a failure", async () => {
    catalogMock.mockResolvedValue([]);
    mount();
    await vi.waitFor(() => expect(maybe("[data-mcp-content-empty]")).not.toBeNull());

    expect(dialog().textContent).toContain("No ready external server offers resources or prompts.");
  });

  it("shows the catalog's own failure with its code", async () => {
    catalogMock.mockRejectedValue({
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      origin: { subsystem: "app" },
    });
    mount();
    await vi.waitFor(() => expect(maybe("[data-mcp-content-error]")).not.toBeNull());

    const failure = find("[data-mcp-content-error]");
    expect(failure.textContent).toContain("APP-INTERNAL");
  });
});

describe("reading and attaching a resource", () => {
  it("draws the exact text the server returned and attaches that same text", async () => {
    const text = "line one\nline two\n\tindented";
    readMock.mockResolvedValue(
      read({ contents: [{ uri: "file:///welcome.txt", mimeType: "text/plain", text, truncated: false }] }),
    );
    mount();
    await vi.waitFor(() => expect(maybe('[data-mcp-resource="file:///welcome.txt"]')).not.toBeNull());

    click('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-read]');
    await vi.waitFor(() => expect(maybe("[data-mcp-content-preview]")).not.toBeNull());

    expect(readMock).toHaveBeenCalledWith("files", "file:///welcome.txt");
    // Not a summary or a re-wrap: the preview holds the returned string itself.
    expect(find("[data-mcp-content-preview]").querySelector("pre")?.textContent).toBe(text);

    const attach = find<HTMLButtonElement>('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-attach]');
    expect(attach.disabled).toBe(false);
    attach.click();
    await settle();

    const assistant = useAssistantStore();
    expect(assistant.attachments).toEqual([
      { server: "files", uri: "file:///welcome.txt", name: "Welcome", mimeType: "text/plain", text },
    ]);
  });

  it("previews an image and refuses to attach a blob", async () => {
    readMock.mockResolvedValue(
      read({
        uri: "file:///logo.png",
        contents: [{ uri: "file:///logo.png", mimeType: "image/png", blob: "aGVsbG8=", truncated: false }],
      }),
    );
    mount();
    await vi.waitFor(() => expect(maybe('[data-mcp-resource="file:///logo.png"]')).not.toBeNull());

    click('[data-mcp-resource="file:///logo.png"] [data-mcp-resource-read]');
    await vi.waitFor(() => expect(maybe("[data-mcp-content-preview] img")).not.toBeNull());

    expect(find<HTMLImageElement>("[data-mcp-content-preview] img").src).toBe("data:image/png;base64,aGVsbG8=");
    expect(find<HTMLButtonElement>('[data-mcp-resource="file:///logo.png"] [data-mcp-resource-attach]').disabled).toBe(
      true,
    );
  });

  it("names a non-image blob as binary rather than pretending to show it", async () => {
    readMock.mockResolvedValue(
      read({
        uri: "file:///logo.png",
        contents: [{ uri: "file:///logo.png", mimeType: "application/pdf", blob: "aGVsbG8=", truncated: false }],
      }),
    );
    mount();
    await vi.waitFor(() => expect(maybe('[data-mcp-resource="file:///logo.png"]')).not.toBeNull());

    click('[data-mcp-resource="file:///logo.png"] [data-mcp-resource-read]');
    await vi.waitFor(() => expect(maybe("[data-mcp-content-preview]")).not.toBeNull());

    const preview = find("[data-mcp-content-preview]");
    expect(preview.textContent).toContain("Binary content");
    expect(preview.textContent).toContain("application/pdf");
    expect(preview.querySelector("img")).toBeNull();
  });

  it("says why a read failed and offers no attach", async () => {
    readMock.mockRejectedValue({
      version: 1,
      code: "MCP-SERVER-NOT-FOUND",
      messageKey: "errors.mcp.serverNotFound",
      origin: { subsystem: "app" },
    });
    mount();
    await vi.waitFor(() => expect(maybe('[data-mcp-resource="file:///welcome.txt"]')).not.toBeNull());

    click('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-read]');
    await vi.waitFor(() =>
      expect(find('[data-mcp-resource="file:///welcome.txt"]').textContent).toContain("MCP-SERVER-NOT-FOUND"),
    );

    expect(maybe('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-attach]')).toBeNull();
  });
});

describe("rendering and inserting a prompt", () => {
  it("renders with the typed arguments and inserts exactly what was previewed", async () => {
    mount();
    await vi.waitFor(() => expect(maybe("[data-mcp-content-tab]")).not.toBeNull());

    click('[data-mcp-content-tab="prompts"]');
    await settle();
    const row = '[data-mcp-prompt="files:summarize"]';
    await vi.waitFor(() => expect(maybe(row)).not.toBeNull());

    await type(`${row} [data-mcp-prompt-argument="topic"]`, "networks");
    click(`${row} [data-mcp-prompt-render]`);
    await vi.waitFor(() => expect(maybe(`${row} [data-mcp-prompt-preview]`)).not.toBeNull());

    expect(promptMock).toHaveBeenCalledWith("files", "summarize", { topic: "networks", tone: "" });
    const preview = find(`${row} [data-mcp-prompt-preview]`).textContent ?? "";
    expect(preview).toBe("user: Summarize networks\n\nassistant: Noted.");

    click(`${row} [data-mcp-prompt-insert]`);
    await settle();

    expect(inserted).toEqual(["user: Summarize networks\n\nassistant: Noted."]);
  });

  it("refuses to render while a required argument is empty", async () => {
    mount();
    await vi.waitFor(() => expect(maybe("[data-mcp-content-tab]")).not.toBeNull());

    click('[data-mcp-content-tab="prompts"]');
    await settle();
    const row = '[data-mcp-prompt="files:summarize"]';
    await vi.waitFor(() => expect(maybe(row)).not.toBeNull());

    click(`${row} [data-mcp-prompt-render]`);
    await vi.waitFor(() => expect(find(row).textContent).toContain("Fill in the required arguments first"));

    expect(promptMock).not.toHaveBeenCalled();
    expect(maybe(`${row} [data-mcp-prompt-insert]`)).toBeNull();
  });

  it("shows a failed render with its code and inserts nothing", async () => {
    promptMock.mockRejectedValue({
      version: 1,
      code: "MCP-SERVER-NOT-FOUND",
      messageKey: "errors.mcp.serverNotFound",
      origin: { subsystem: "app" },
    });
    mount();
    await vi.waitFor(() => expect(maybe("[data-mcp-content-tab]")).not.toBeNull());

    click('[data-mcp-content-tab="prompts"]');
    await settle();
    const row = '[data-mcp-prompt="files:summarize"]';
    await vi.waitFor(() => expect(maybe(row)).not.toBeNull());

    await type(`${row} [data-mcp-prompt-argument="topic"]`, "networks");
    click(`${row} [data-mcp-prompt-render]`);
    await vi.waitFor(() => expect(find(row).textContent).toContain("MCP-SERVER-NOT-FOUND"));

    expect(maybe(`${row} [data-mcp-prompt-insert]`)).toBeNull();
    expect(inserted).toEqual([]);
  });
});

describe("a full attachment list", () => {
  it("says the fifth resource was refused rather than silently dropping it", async () => {
    mount();
    const assistant = useAssistantStore();
    for (let index = 0; index < 4; index += 1) {
      assistant.attachResource({ server: `s${index}`, uri: `file:///${index}`, name: `${index}`, text: "x" });
    }
    await vi.waitFor(() => expect(maybe('[data-mcp-resource="file:///welcome.txt"]')).not.toBeNull());

    click('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-read]');
    await vi.waitFor(() => expect(maybe("[data-mcp-content-preview]")).not.toBeNull());
    click('[data-mcp-resource="file:///welcome.txt"] [data-mcp-resource-attach]');
    await settle();

    expect(assistant.attachments).toHaveLength(4);
    expect(maybe("[data-mcp-content-full]")).not.toBeNull();
  });
});
