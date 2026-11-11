/**
 * The agent interface's store, as the window uses it.
 *
 * The three things worth holding still are the three that would be quietly wrong on screen: a
 * status read from the listener rather than from the settings file, a question that stays until it
 * is answered, and an audit that is replaced from the file rather than accumulated.
 */
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createPinia, setActivePinia } from "pinia";
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
}));

import {
  mcpApprovalAnswer,
  mcpAuditTail,
  mcpClientConfig,
  mcpDisable,
  mcpEnable,
  mcpStatus,
  onMcpApproval,
  onMcpStatus,
} from "@/lib/backend/tauri";
import {
  MCP_AUDIT_STATUS_LABEL_KEYS,
  MCP_SCOPE_HINT_KEYS,
  MCP_SCOPE_LABEL_KEYS,
  RISK_LEVEL_LABEL_KEYS,
} from "@/lib/mcp/labels";
import { useMcpStore } from "@/stores/mcp";
import type { McpApprovalRequest, McpAuditEntry, McpStatus } from "@/types/mcp";

const statusMock = vi.mocked(mcpStatus);
const enableMock = vi.mocked(mcpEnable);
const disableMock = vi.mocked(mcpDisable);
const configMock = vi.mocked(mcpClientConfig);
const auditMock = vi.mocked(mcpAuditTail);
const answerMock = vi.mocked(mcpApprovalAnswer);
const approvalEventMock = vi.mocked(onMcpApproval);
const statusEventMock = vi.mocked(onMcpStatus);

const LOCALES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../i18n/locales");

/** One listener, and still off — the state this page must not draw as "on". */
function stopped(overrides: Partial<McpStatus> = {}): McpStatus {
  return {
    enabled: false,
    port: null,
    scope: "readOnly",
    clientConfig: '{"mcpServers":{"7shell":{"command":"sshell-mcp"}}}',
    auditPath: "/config/audit/mcp-audit.jsonl",
    pendingApprovals: 0,
    ...overrides,
  };
}

/** A destructive call waiting to be allowed. */
const approving: McpApprovalRequest = {
  callId: "call-7",
  tool: "sshell_exec_run",
  sessionId: "session-1",
  sessionName: "lab",
  subject: "rm -rf /tmp/seven-shell-scratch",
  reason: {
    kind: "risk",
    assessment: {
      level: "destructive",
      reasons: [{ ruleId: "recursive-force-delete", labelKey: "risk.recursiveForceDelete", level: "destructive" }],
    },
  },
  expiresAt: 1_780_000_120_000,
};

const audited: McpAuditEntry = {
  at: 1_780_000_000_000,
  tool: "sshell_sessions_list",
  scope: "readOnly",
  status: "succeeded",
  durationMs: 12,
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  approvalEventMock.mockResolvedValue(() => {});
  statusEventMock.mockResolvedValue(() => {});
  statusMock.mockResolvedValue(stopped());
  auditMock.mockResolvedValue([]);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the agent interface as the window watches it", () => {
  it("reads whether it is running from the listener, not from a switch", async () => {
    statusMock.mockResolvedValue(stopped({ enabled: true, port: 51234, pendingApprovals: 1 }));
    const store = useMcpStore();

    await store.refresh();

    expect(store.enabled).toBe(true);
    expect(store.status?.port).toBe(51234);
    expect(store.status?.pendingApprovals).toBe(1);
    expect(store.failure).toBeNull();
  });

  it("takes the listener's answer when it is started or stopped", async () => {
    const store = useMcpStore();
    enableMock.mockResolvedValue(stopped({ enabled: true, port: 51234 }));
    disableMock.mockResolvedValue(stopped());

    expect(await store.enable()).toBe(true);
    expect(store.status?.port).toBe(51234);
    expect(await store.disable()).toBe(true);
    expect(store.status?.enabled).toBe(false);
  });

  it("keeps a failed start apart from a stopped listener", async () => {
    const store = useMcpStore();
    enableMock.mockRejectedValue({
      version: 1,
      code: "MCP-START-FAILED",
      messageKey: "errors.store.io",
      origin: { subsystem: "app" },
    });

    expect(await store.enable()).toBe(false);
    // The status is untouched: nothing claims the listener is off because a start failed either.
    expect(store.status).toBeNull();
    expect(store.failure?.code).toBe("MCP-START-FAILED");
  });

  it("replaces the audit tail rather than appending to what it saw", async () => {
    const store = useMcpStore();
    auditMock.mockResolvedValueOnce([audited]);
    await store.loadAudit();
    expect(store.audit).toEqual([audited]);

    // A second read is the file again — one line, because that is what the file holds now.
    auditMock.mockResolvedValueOnce([{ ...audited, tool: "sshell_sftp_list" }]);
    await store.loadAudit(5);

    expect(store.audit).toHaveLength(1);
    expect(store.audit[0]?.tool).toBe("sshell_sftp_list");
    expect(auditMock).toHaveBeenLastCalledWith(5);
    expect(store.auditLoaded).toBe(true);
  });

  it("keeps the question up when the answer did not arrive", async () => {
    const store = useMcpStore();
    store.pendingApproval = approving;
    answerMock.mockRejectedValue({
      version: 1,
      code: "MCP-APPROVAL-GONE",
      messageKey: "errors.store.io",
      origin: { subsystem: "app" },
    });

    expect(await store.answerApproval("call-7", "denied")).toBe(false);

    expect(store.pendingApproval?.callId).toBe("call-7");
    expect(store.failure?.code).toBe("MCP-APPROVAL-GONE");
  });

  it("clears the question once the answer has been delivered", async () => {
    const store = useMcpStore();
    store.pendingApproval = approving;
    answerMock.mockResolvedValue(undefined);

    expect(await store.answerApproval("call-7", "approved")).toBe(true);

    expect(store.pendingApproval).toBeNull();
    expect(answerMock).toHaveBeenCalledWith("call-7", "approved");
  });

  it("drops a question whose deadline passed, and only that one", () => {
    const store = useMcpStore();
    store.pendingApproval = approving;

    store.forgetApproval("another-call");
    expect(store.pendingApproval?.callId).toBe("call-7");

    store.forgetApproval("call-7");
    expect(store.pendingApproval).toBeNull();
  });

  it("draws the question and the status the listener pushes", async () => {
    const store = useMcpStore();
    let asked: ((request: McpApprovalRequest) => void) | undefined;
    let changed: ((next: McpStatus) => void) | undefined;
    approvalEventMock.mockImplementation(async (listener) => {
      asked = listener;
      return () => {};
    });
    statusEventMock.mockImplementation(async (listener) => {
      changed = listener;
      return () => {};
    });

    const stop = store.initialize();
    await vi.waitFor(() => expect(asked).toBeDefined());

    asked?.(approving);
    expect(store.pendingApproval?.callId).toBe("call-7");

    changed?.(stopped({ enabled: true, port: 51234 }));
    expect(store.status?.enabled).toBe(true);

    // The question goes away again when the call is answered and the listener says so.
    changed?.(stopped());
    expect(store.status?.enabled).toBe(false);
    stop();
  });

  it("drops both subscriptions when the window goes away", async () => {
    const store = useMcpStore();
    const stopApproval = vi.fn();
    const stopStatus = vi.fn();
    approvalEventMock.mockResolvedValue(stopApproval);
    statusEventMock.mockResolvedValue(stopStatus);

    const stop = store.initialize();
    await vi.waitFor(() => {
      expect(approvalEventMock).toHaveBeenCalled();
      expect(statusEventMock).toHaveBeenCalled();
    });
    // One more turn, so the store has been handed both unsubscribe functions before they are used.
    await Promise.resolve();
    stop();

    expect(stopApproval).toHaveBeenCalled();
    expect(stopStatus).toHaveBeenCalled();
  });

  it("copies the client configuration instead of keeping it on screen", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const store = useMcpStore();
    configMock.mockResolvedValue('{"mcpServers":{"7shell":{"command":"sshell-mcp"}}}');

    expect(await store.copyClientConfig()).toBe(true);

    expect(writeText).toHaveBeenCalledWith('{"mcpServers":{"7shell":{"command":"sshell-mcp"}}}');
    // Nothing about the configuration is held by the store: what is copied is not what is drawn.
    expect(store.copyFailed).toBe(false);
    expect(JSON.stringify(store.$state)).not.toContain("mcpServers");
  });

  it("says so when the clipboard refused the configuration", async () => {
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText: vi.fn(async () => {
          throw new Error("the clipboard is busy");
        }),
      },
    });
    const store = useMcpStore();
    configMock.mockResolvedValue("{}");

    expect(await store.copyClientConfig()).toBe(false);
    expect(store.copyFailed).toBe(true);
    expect(store.failure).not.toBeNull();
  });
});

/**
 * The keys the page and the dialog name through tables, which `check-i18n.mjs` cannot read.
 *
 * The same guard `rows.spec.ts` puts on the settings table: a key with a typo in it would be drawn
 * as the key itself, and no gate would fail.
 */
describe("the words the agent interface draws", () => {
  const languages = ["en", "zh-CN"];

  function messages(language: string): Set<string> {
    const out = new Set<string>();
    const directory = path.join(LOCALES, language);
    for (const name of readdirSync(directory)) {
      if (!name.endsWith(".json")) {
        continue;
      }
      const flat = (value: unknown, prefix: string): void => {
        for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
          const full = prefix ? `${prefix}.${key}` : key;
          if (entry !== null && typeof entry === "object") {
            flat(entry, full);
          } else {
            out.add(full);
          }
        }
      };
      flat(JSON.parse(readFileSync(path.join(directory, name), "utf8")), "");
    }
    return out;
  }

  it("names every risk level the dialog shows, in every language", () => {
    const missing: string[] = [];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of Object.values(RISK_LEVEL_LABEL_KEYS)) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("names every tier and its explanation, in every language", () => {
    const missing: string[] = [];
    const keys = [
      ...Object.values(MCP_SCOPE_LABEL_KEYS),
      ...Object.values(MCP_SCOPE_HINT_KEYS),
      ...Object.values(MCP_AUDIT_STATUS_LABEL_KEYS),
      "settings.nav.mcp",
    ];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of keys) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });
});
