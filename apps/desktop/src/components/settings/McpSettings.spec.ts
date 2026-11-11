/**
 * The external-server block on the agent page.
 *
 * The store and the host are exercised elsewhere; what is asserted here is what the page itself
 * would get wrong — a form that saves an id the shell refuses, a secrets box that draws a value
 * back, a row that says "ready" while the host says it failed.
 */
import { createPinia, setActivePinia } from "pinia";
import { TooltipProvider } from "reka-ui";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  mcpStatus: vi.fn(),
  mcpEnable: vi.fn(),
  mcpDisable: vi.fn(),
  mcpClientConfig: vi.fn(),
  mcpAuditTail: vi.fn(),
  mcpApprovalAnswer: vi.fn(),
  onMcpApproval: vi.fn(),
  onMcpStatus: vi.fn(),
  mcpServersList: vi.fn(),
  mcpServerSave: vi.fn(),
  mcpServerRemove: vi.fn(),
  mcpServerSecretsSet: vi.fn(),
  mcpServerRefresh: vi.fn(),
  onMcpServerStatus: vi.fn(),
}));

import McpSettings from "@/components/settings/McpSettings.vue";
import { i18n } from "@/i18n";
import {
  mcpAuditTail,
  mcpServerRefresh,
  mcpServerRemove,
  mcpServerSave,
  mcpServerSecretsSet,
  mcpServersList,
  mcpStatus,
} from "@/lib/backend/tauri";
import type { McpServerConfig, McpServerState, McpServerStatus, McpServerView } from "@/types/mcp";

const serversMock = vi.mocked(mcpServersList);
const saveMock = vi.mocked(mcpServerSave);
const removeMock = vi.mocked(mcpServerRemove);
const secretsMock = vi.mocked(mcpServerSecretsSet);
const refreshMock = vi.mocked(mcpServerRefresh);
const statusMock = vi.mocked(mcpStatus);
const auditMock = vi.mocked(mcpAuditTail);

let app: App | undefined;
let host: HTMLDivElement;

const SERVER_CONFIG: McpServerConfig = {
  id: "files",
  name: "Files",
  transport: "stdio",
  command: "npx",
  args: ["-y", "server-files"],
  enabled: true,
  toolTimeoutSeconds: 60,
};

/** One server as the host answers it. */
function view(
  id: string,
  options: {
    state?: McpServerState;
    transport?: McpServerConfig["transport"];
    tools?: number;
    hasSecrets?: boolean | null;
    config?: Partial<McpServerConfig>;
    status?: Partial<McpServerStatus>;
  } = {},
): McpServerView {
  return {
    config: { ...SERVER_CONFIG, id, name: id, transport: options.transport ?? "stdio", ...options.config },
    status: { serverId: id, state: options.state ?? "ready", tools: options.tools ?? 3, ...options.status },
    // `null` is a real answer — the vault is locked — so only an absent one reads as "none".
    hasSecrets: options.hasSecrets === undefined ? false : options.hasSecrets,
  };
}

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  // The page provides the tooltip context in the application; here the block is mounted alone,
  // so the provider its help icons need is supplied by the harness.
  app = createApp({ render: () => h(TooltipProvider, null, { default: () => h(McpSettings) }) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await vi.waitFor(() => expect(serversMock).toHaveBeenCalled());
  await nextTick();
}

/** One turn of the event loop, which is what reka-ui's dismiss layer is registered on. */
async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function field(id: string): HTMLInputElement | HTMLTextAreaElement | null {
  return host.querySelector<HTMLInputElement | HTMLTextAreaElement>(`#${id}`);
}

/** Types into a field the way a person does, so the draft is what changes. */
async function type(id: string, value: string): Promise<void> {
  const element = field(id);
  if (!element) {
    throw new Error(`no field ${id}`);
  }
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await nextTick();
}

async function press(selector: string, where: ParentNode = host): Promise<void> {
  const button = where.querySelector<HTMLButtonElement>(selector);
  if (!button) {
    throw new Error(`no button ${selector}`);
  }
  button.click();
  await nextTick();
}

function saveButton(): HTMLButtonElement {
  const element = host.querySelector<HTMLButtonElement>("[data-mcp-server-save]");
  if (!element) {
    throw new Error("the form has no save button");
  }
  return element;
}

/** The form's transport dropdown, which is the only one carrying that id. */
function transportTrigger(): HTMLElement {
  const element = host.querySelector<HTMLElement>("#mcp-server-transport");
  if (!element) {
    throw new Error("the form has no transport dropdown");
  }
  return element;
}

async function chooseTransport(label: string): Promise<void> {
  transportTrigger().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
  transportTrigger().click();
  await settle();
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!option) {
    throw new Error(`the list has no ${label} entry`);
  }
  option.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "mouse" }));
  await settle();
}

function row(id: string): HTMLElement {
  const element = host.querySelector<HTMLElement>(`[data-mcp-server-row][data-mcp-server-id="${id}"]`);
  if (!element) {
    throw new Error(`no row for ${id}`);
  }
  return element;
}

function typeSecret(where: HTMLElement, key: string, value: string): void {
  const inputs = where.querySelectorAll<HTMLInputElement>("input");
  const keyInput = inputs[0];
  const valueInput = inputs[1];
  if (!keyInput || !valueInput) {
    throw new Error("the secrets row has no name and value boxes");
  }
  keyInput.value = key;
  keyInput.dispatchEvent(new Event("input"));
  valueInput.value = value;
  valueInput.dispatchEvent(new Event("input"));
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  statusMock.mockResolvedValue({
    enabled: false,
    port: null,
    scope: "readOnly",
    clientConfig: "{}",
    auditPath: "/config/audit.jsonl",
    pendingApprovals: 0,
  });
  auditMock.mockResolvedValue([]);
  serversMock.mockResolvedValue([]);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="listbox"], [role="dialog"]')) {
    node.remove();
  }
});

describe("the external servers the page draws", () => {
  it("says so when nothing is configured", async () => {
    await mount();

    expect(host.querySelectorAll("[data-mcp-server-row]").length).toBe(0);
    expect(host.textContent).toContain("No external servers are configured.");
  });

  it("draws each server's name, transport, state, tool count and secret state", async () => {
    serversMock.mockResolvedValue([
      view("files", { config: { name: "Files" }, tools: 4, state: "ready", hasSecrets: false }),
      view("search", {
        transport: "http",
        state: "failed",
        tools: 0,
        hasSecrets: null,
        status: { code: "MCP-EXTERNAL-FAILED", message: "the connection was refused" },
      }),
    ]);
    await mount();

    expect(host.querySelectorAll("[data-mcp-server-row]").length).toBe(2);
    expect(row("files").textContent).toContain("Files");
    expect(row("files").textContent).toContain("4 tools");
    expect(row("files").textContent).toContain("Ready");
    expect(row("files").textContent).toContain("No secrets saved");

    expect(row("search").textContent).toContain("HTTP");
    expect(row("search").textContent).toContain("Failed");
    expect(row("search").textContent).toContain("MCP-EXTERNAL-FAILED");
    expect(row("search").textContent).toContain("the connection was refused");
    // A locked vault is "cannot tell", never "none saved": the two answers invite different actions.
    expect(row("search").textContent).toContain("Vault locked");
  });
});

describe("the server form", () => {
  it("will not save an id the shell would refuse, or a stdio server with no command", async () => {
    serversMock.mockResolvedValue([view("files", { config: { name: "Files" } })]);
    await mount();
    await press("[data-mcp-server-add]");

    expect(saveButton().disabled, "nothing is filled in yet").toBe(true);

    await type("mcp-server-id", "a__b");
    await type("mcp-server-command", "npx");
    expect(saveButton().disabled).toBe(true);
    expect(host.textContent).toContain("no double underscore");

    // An id another row already has is refused before the round trip that would refuse it.
    await type("mcp-server-id", "files");
    expect(saveButton().disabled).toBe(true);
    expect(host.textContent).toContain("Another server already uses this identifier.");

    await type("mcp-server-id", "files-2");
    expect(saveButton().disabled).toBe(false);

    await type("mcp-server-command", "");
    expect(saveButton().disabled, "a stdio server with no program is not one").toBe(true);
    expect(host.textContent).toContain("A stdio server needs a command.");
  });

  it("saves the configuration with the arguments one per line", async () => {
    await mount();
    await press("[data-mcp-server-add]");

    await type("mcp-server-id", "files");
    await type("mcp-server-name", "Files");
    await type("mcp-server-command", "npx");
    await type("mcp-server-args", "-y\nserver-files\n");
    saveMock.mockResolvedValue(view("files"));

    await press("[data-mcp-server-save]");
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));

    expect(saveMock).toHaveBeenCalledWith({
      id: "files",
      name: "Files",
      transport: "stdio",
      command: "npx",
      args: ["-y", "server-files"],
      url: undefined,
      enabled: true,
      toolTimeoutSeconds: 60,
    });
    // The host's answer is the row, so the form closes and the list draws what it said.
    await vi.waitFor(() => expect(host.querySelector("[data-mcp-server-row]")).not.toBeNull());
  });

  it("needs a URL once the transport is HTTP", async () => {
    await mount();
    await press("[data-mcp-server-add]");
    await type("mcp-server-id", "search");

    expect(saveButton().disabled, "no stdio command yet").toBe(true);

    await chooseTransport("HTTP");
    expect(saveButton().disabled, "no URL yet").toBe(true);
    expect(host.textContent).toContain("An HTTP or SSE server needs a URL.");

    await type("mcp-server-url", "https://example.test/mcp");
    expect(saveButton().disabled).toBe(false);

    saveMock.mockResolvedValue(view("search", { transport: "http" }));
    await press("[data-mcp-server-save]");
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));
    expect(saveMock.mock.calls[0]?.[0]).toMatchObject({
      id: "search",
      transport: "http",
      command: undefined,
      args: [],
      url: "https://example.test/mcp",
    });
  });
});

describe("the secrets editor", () => {
  it("starts empty, saves both maps and does not draw the values back", async () => {
    serversMock.mockResolvedValue([view("files", { hasSecrets: false })]);
    await mount();

    await press("[data-mcp-server-secrets]");
    const envRow = host.querySelector<HTMLElement>("[data-mcp-server-env-row]");
    const headerRow = host.querySelector<HTMLElement>("[data-mcp-server-header-row]");
    if (!envRow || !headerRow) {
      throw new Error("the editor has no environment or headers row");
    }
    // Nothing is filled in from the vault, because nothing can be read back from it.
    expect(envRow.querySelectorAll<HTMLInputElement>("input")[1]?.value).toBe("");

    typeSecret(envRow, "TOKEN", "abc");
    typeSecret(headerRow, "Authorization", "Bearer xyz");
    secretsMock.mockResolvedValue(view("files", { hasSecrets: true }));

    await press("[data-mcp-server-secrets-save]");
    await vi.waitFor(() => expect(secretsMock).toHaveBeenCalledTimes(1));

    expect(secretsMock).toHaveBeenCalledWith("files", { TOKEN: "abc" }, { Authorization: "Bearer xyz" });

    // The boxes are empty again and the row says only that something is held.
    await vi.waitFor(() =>
      expect(host.querySelector<HTMLInputElement>("[data-mcp-server-env-row] [data-secret-value]")?.value).toBe(""),
    );
    await vi.waitFor(() => expect(row("files").textContent).toContain("Secrets saved"));
    expect(host.textContent).toContain("Saved. The values are not shown again.");
    expect(host.textContent).not.toContain("abc");
    expect(host.textContent).not.toContain("Bearer xyz");
  });

  it("clears what is stored when the editor is saved empty", async () => {
    serversMock.mockResolvedValue([view("files", { hasSecrets: true })]);
    await mount();

    await press("[data-mcp-server-secrets]");
    secretsMock.mockResolvedValue(view("files", { hasSecrets: false }));

    await press("[data-mcp-server-secrets-save]");
    await vi.waitFor(() => expect(secretsMock).toHaveBeenCalledTimes(1));

    expect(secretsMock).toHaveBeenCalledWith("files", {}, {});
    await vi.waitFor(() => expect(host.textContent).toContain("Cleared. Nothing is stored for this server."));
  });

  it("refuses a value with no name instead of sending it", async () => {
    serversMock.mockResolvedValue([view("files")]);
    await mount();

    await press("[data-mcp-server-secrets]");
    const envRow = host.querySelector<HTMLElement>("[data-mcp-server-env-row]");
    if (!envRow) {
      throw new Error("the editor has no environment row");
    }
    const valueInput = envRow.querySelectorAll<HTMLInputElement>("input")[1];
    valueInput!.value = "orphan";
    valueInput!.dispatchEvent(new Event("input"));
    await nextTick();

    expect(host.querySelector<HTMLButtonElement>("[data-mcp-server-secrets-save]")?.disabled).toBe(true);
    expect(host.textContent).toContain("Every entry needs a name");
  });
});

describe("the row's own actions", () => {
  it("turns one server off through a write of its own", async () => {
    serversMock.mockResolvedValue([view("files", { config: { name: "Files" } })]);
    saveMock.mockResolvedValue(view("files", { config: { name: "Files", enabled: false } }));
    await mount();

    const control = row("files").querySelector<HTMLElement>('[role="switch"]');
    control?.click();
    await vi.waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));

    expect(saveMock).toHaveBeenCalledWith(expect.objectContaining({ id: "files", enabled: false }));
  });

  it("reconnects only the server whose button was pressed", async () => {
    serversMock.mockResolvedValue([view("files"), view("search")]);
    refreshMock.mockResolvedValue(view("search", { state: "ready", tools: 2 }));
    await mount();

    await press('[data-mcp-server-id="search"] [data-mcp-server-refresh]');
    await vi.waitFor(() => expect(refreshMock).toHaveBeenCalledTimes(1));

    expect(refreshMock).toHaveBeenCalledWith("search");
    expect(refreshMock).not.toHaveBeenCalledWith("files");
  });

  it("asks before removing, then removes through the host", async () => {
    serversMock.mockResolvedValue([view("files")]);
    removeMock.mockResolvedValue(undefined);
    await mount();

    await press('[data-mcp-server-id="files"] [data-mcp-server-remove]');
    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    if (!dialog) {
      throw new Error("the page did not ask about the removal");
    }
    const confirm = [...dialog.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Remove");
    if (!confirm) {
      throw new Error("the question has no confirm button");
    }
    confirm.click();
    await vi.waitFor(() => expect(removeMock).toHaveBeenCalledWith("files"));

    await vi.waitFor(() => expect(host.querySelectorAll("[data-mcp-server-row]").length).toBe(0));
  });
});
