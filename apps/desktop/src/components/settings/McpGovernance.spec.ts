/**
 * The three Agent access pages, as the window draws them.
 *
 * The store and the host are exercised elsewhere; what is asserted here is what this page would
 * get wrong on its own — a tab that resets a draft it was supposed to keep, a hidden session that
 * writes a filter where the host reads an allowlist, and an "effective tier" line that names a tier
 * without saying which rule chose it.
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
  mcpCatalog: vi.fn(),
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
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
  sessionsList: vi.fn(),
  groupsList: vi.fn(),
}));

import McpSettings from "@/components/settings/McpSettings.vue";
import { i18n } from "@/i18n";
import { groupsList, mcpCatalog, mcpServersList, mcpStatus, sessionsList, settingsSet } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import type { McpCatalog, McpCatalogTool } from "@/types/mcp";
import type { Session, SessionGroup } from "@/types/session";

const statusMock = vi.mocked(mcpStatus);
const serversMock = vi.mocked(mcpServersList);
const catalogMock = vi.mocked(mcpCatalog);
const sessionsMock = vi.mocked(sessionsList);
const groupsMock = vi.mocked(groupsList);
const settingsSetMock = vi.mocked(settingsSet);

let app: App | undefined;
let host: HTMLDivElement;

/** One tool as the catalog answers it, with everything on unless a case says otherwise. */
function tool(name: string, overrides: Partial<McpCatalogTool> = {}): McpCatalogTool {
  return {
    name,
    title: name,
    description: `${name} description`,
    access: "read",
    risk: "read",
    category: "sessions",
    enabled: true,
    source: null,
    ...overrides,
  };
}

const ALL_ON: McpCatalog = {
  tools: [
    tool("sshell_sessions_list", { title: "List sessions", category: "sessions" }),
    tool("sshell_exec_run", {
      title: "Run a command",
      category: "exec",
      access: "write",
      risk: "commandGraded",
    }),
    tool("sshell_sftp_write_text", {
      title: "Write remote text",
      category: "files",
      access: "write",
      risk: "write",
    }),
    tool("mcp__files__read", {
      title: "Read a shared file",
      category: "external",
      enabled: true,
      source: "files",
    }),
  ],
  counts: { total: 4, enabled: 4 },
};

/** The same catalog after the host was told `sshell_exec_run` is off. */
const EXEC_OFF: McpCatalog = {
  tools: ALL_ON.tools.map((entry) => (entry.name === "sshell_exec_run" ? { ...entry, enabled: false } : entry)),
  counts: { total: 4, enabled: 3 },
};

function session(id: string, name: string, groupId?: string): Session {
  return {
    id,
    name,
    groupId,
    protocol: "ssh",
    host: "example.test",
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: true },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
    updatedAt: 0,
  };
}

const SESSIONS: Session[] = [
  session("prod-web", "prod-web", "prod"),
  session("prod-db", "prod-db", "prod"),
  session("dev-box", "dev-box"),
];

const GROUPS: SessionGroup[] = [{ id: "prod", name: "Prod", order: 0, pinned: false }];

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  // The page provides the tooltip context in the application; here it is mounted alone, so the
  // provider its help icons need is supplied by the harness.
  app = createApp({ render: () => h(TooltipProvider, null, { default: () => h(McpSettings) }) });
  app.use(i18n);
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

async function press(selector: string): Promise<void> {
  const button = host.querySelector<HTMLButtonElement>(selector);
  if (!button) {
    throw new Error(`no button ${selector}`);
  }
  button.click();
  await settle();
}

/** Whether a panel is actually drawn: `v-show` keeps the other two mounted but display:none. */
function panelVisible(name: string): boolean {
  const panel = host.querySelector<HTMLElement>(`[data-mcp-panel="${name}"]`);
  return panel !== null && panel.style.display !== "none";
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  statusMock.mockResolvedValue({
    enabled: false,
    port: null,
    scope: "readOnly",
    clientConfig: '{"mcpServers":{"7shell":{"command":"sshell-mcp"}}}',
    auditPath: "/config/audit.jsonl",
    pendingApprovals: 0,
  });
  serversMock.mockResolvedValue([]);
  catalogMock.mockResolvedValue(ALL_ON);
  sessionsMock.mockResolvedValue(SESSIONS);
  groupsMock.mockResolvedValue(GROUPS);
  settingsSetMock.mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="listbox"], [role="dialog"]')) {
    node.remove();
  }
});

describe("the three pages", () => {
  it("switches panels and keeps the one that was left, instead of resetting it", async () => {
    await mount();
    expect(panelVisible("access")).toBe(true);
    expect(host.querySelector('[data-mcp-tab="access"]')?.getAttribute("aria-selected")).toBe("true");
    expect(panelVisible("capabilities")).toBe(false);

    await press('[data-mcp-tab="capabilities"]');
    await vi.waitFor(() => expect(catalogMock).toHaveBeenCalled());
    await vi.waitFor(() => expect(host.querySelector('[data-mcp-tool-name="sshell_exec_run"]')).not.toBeNull());
    expect(panelVisible("access")).toBe(false);
    expect(panelVisible("capabilities")).toBe(true);
    expect(host.querySelector('[data-mcp-tab="capabilities"]')?.getAttribute("aria-selected")).toBe("true");

    // A draft made here survives a visit to the access page.
    host.querySelector<HTMLElement>('[data-mcp-tool-name="sshell_exec_run"] [data-mcp-tool-switch]')?.click();
    await settle();
    await press('[data-mcp-tab="access"]');
    await press('[data-mcp-tab="capabilities"]');
    expect(host.querySelector('[data-mcp-tool-name="sshell_exec_run"]')?.getAttribute("data-mcp-tool-enabled")).toBe(
      "false",
    );

    await press('[data-mcp-tab="permissions"]');
    await vi.waitFor(() => expect(sessionsMock).toHaveBeenCalled());
    await vi.waitFor(() => expect(host.querySelector('[data-mcp-session-id="prod-web"]')).not.toBeNull());
    expect(panelVisible("permissions")).toBe(true);
    expect(panelVisible("capabilities")).toBe(false);
  });
});

describe("the capabilities page", () => {
  it("writes a switched-off tool into disabledTools and reloads the catalog", async () => {
    catalogMock.mockResolvedValueOnce(ALL_ON).mockResolvedValueOnce(EXEC_OFF);
    await mount();
    await press('[data-mcp-tab="capabilities"]');
    await vi.waitFor(() => expect(host.querySelector('[data-mcp-tool-name="sshell_exec_run"]')).not.toBeNull());

    const row = host.querySelector<HTMLElement>('[data-mcp-tool-name="sshell_exec_run"]');
    row?.querySelector<HTMLElement>("[data-mcp-tool-switch]")?.click();
    await settle();
    expect(row?.getAttribute("data-mcp-tool-enabled"), "the switch is a draft").toBe("false");
    // Nothing is written while the switch moves: Apply is what writes.
    expect(settingsSetMock).not.toHaveBeenCalled();

    await press('[data-mcp-panel="capabilities"] [data-mcp-apply]');
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));

    const payload = settingsSetMock.mock.calls[0]?.[0];
    expect(payload?.mcpToolPolicy).toEqual({ disabledTools: ["sshell_exec_run"] });
    // The policy travels with every save, so the other page's answers must not be dropped.
    expect(payload?.mcpAccessPolicy).toEqual({ visibleSessionIds: null, sessionScopes: {}, groupScopes: {} });

    await vi.waitFor(() => expect(catalogMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(host.textContent).toContain("Applied. The tool list an agent sees is updated."));
    expect(host.querySelector('[data-mcp-tool-name="sshell_exec_run"]')?.getAttribute("data-mcp-tool-enabled")).toBe(
      "false",
    );
  });
});

describe("the permissions page", () => {
  it("hides a session by materialising the allowlist, then writes it", async () => {
    await mount();
    await press('[data-mcp-tab="permissions"]');
    await vi.waitFor(() => expect(host.querySelector('[data-mcp-session-id="prod-web"]')).not.toBeNull());

    expect(host.querySelector("[data-mcp-allowlist-hint]"), "nothing is hidden yet").toBeNull();
    const visibility = host.querySelector<HTMLInputElement>('[data-mcp-session-id="prod-web"] [data-mcp-visibility]');
    expect(visibility?.checked).toBe(true);
    visibility?.click();
    await settle();

    // The first hide turns "every session" into the list that exists now, minus the hidden one.
    expect(host.querySelector("[data-mcp-allowlist-hint]")).not.toBeNull();
    expect(host.querySelector('[data-mcp-session-id="prod-web"] [data-mcp-hidden]')).not.toBeNull();

    await press('[data-mcp-panel="permissions"] [data-mcp-apply]');
    await vi.waitFor(() => expect(settingsSetMock).toHaveBeenCalledTimes(1));

    const payload = settingsSetMock.mock.calls[0]?.[0];
    const visible = [...(payload?.mcpAccessPolicy?.visibleSessionIds ?? [])].sort();
    expect(visible).toEqual(["dev-box", "prod-db"]);
    expect(payload?.mcpScope).toBe("readOnly");
  });

  it("says which tier is in force and which rule chose it", async () => {
    const preferences = usePreferencesStore();
    preferences.mcpScope = "readOnly";
    preferences.mcpAccessPolicy = {
      visibleSessionIds: null,
      sessionScopes: { "prod-db": "highRisk" },
      groupScopes: { prod: "safeWrite" },
    };

    await mount();
    await press('[data-mcp-tab="permissions"]');
    await vi.waitFor(() => expect(host.querySelector('[data-mcp-session-id="prod-web"]')).not.toBeNull());

    const effective = (sessionId: string): HTMLElement | null =>
      host.querySelector<HTMLElement>(`[data-mcp-session-id="${sessionId}"] [data-mcp-effective]`);

    // The group's tier applies to the session that has no setting of its own.
    expect(effective("prod-web")?.textContent).toContain("Writes you allow");
    expect(effective("prod-web")?.textContent).toContain("from group Prod");
    expect(effective("prod-web")?.getAttribute("data-mcp-effective-source")).toBe("group");

    // The session's own setting wins over the group.
    expect(effective("prod-db")?.textContent).toContain("Everything");
    expect(effective("prod-db")?.textContent).toContain("set for this session");
    expect(effective("prod-db")?.getAttribute("data-mcp-effective-source")).toBe("session");

    // Nothing above it: the default applies.
    expect(effective("dev-box")?.textContent).toContain("Read only");
    expect(effective("dev-box")?.textContent).toContain("from the default");
    expect(effective("dev-box")?.getAttribute("data-mcp-effective-source")).toBe("global");

    // The heading carries its own tier, which is what the sessions under it inherit.
    const groupRow = host.querySelector('[data-mcp-permission-kind="group"][data-mcp-group-id="prod"]');
    expect(groupRow?.querySelector("[data-mcp-effective]")?.textContent).toContain("Writes you allow");

    // A card writes the same setting the access tab's dropdown writes: no second draft to diverge.
    await press('[data-mcp-scope-card="safeWrite"]');
    expect(preferences.mcpScope).toBe("safeWrite");
    expect(effective("dev-box")?.textContent).toContain("Writes you allow");
  });
});
