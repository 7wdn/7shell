/**
 * The approval dialog, as the two things around it find it.
 *
 * The question itself is the store's; what is asserted here is what a person — and an end-to-end
 * run — has to be able to tell from the dialog: who asked, why the call stopped, and which external
 * server a tool came from. Those are the stable `data-` handles, because the visible text is
 * translated and the tool name is not enough to say that somebody else wrote it.
 */
import { createPinia, setActivePinia } from "pinia";
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

import ApprovalDialog from "@/components/mcp/ApprovalDialog.vue";
import { i18n } from "@/i18n";
import { mcpApprovalAnswer } from "@/lib/backend/tauri";
import { useMcpStore } from "@/stores/mcp";
import type { McpApprovalRequest } from "@/types/mcp";

const answerMock = vi.mocked(mcpApprovalAnswer);

let app: App | undefined;

/** A tool an external server provides: the dialog can say no more than where it came from. */
const external: McpApprovalRequest = {
  callId: "call-9",
  tool: "mcp__files__read_file",
  reason: { kind: "external", server: "files", tool: "read_file" },
  expiresAt: Date.now() + 120_000,
};

/** A destructive call, asked by this window's own assistant. */
const destructive: McpApprovalRequest = {
  callId: "call-10",
  tool: "sshell_exec_run",
  origin: "ai",
  subject: "rm -rf /tmp/seven-shell-scratch",
  reason: {
    kind: "risk",
    assessment: {
      level: "destructive",
      reasons: [{ ruleId: "recursive-force-delete", labelKey: "risk.recursiveForceDelete", level: "destructive" }],
    },
  },
  expiresAt: Date.now() + 120_000,
};

async function mount(request: McpApprovalRequest): Promise<HTMLElement> {
  const pinia = createPinia();
  setActivePinia(pinia);
  useMcpStore().pendingApproval = request;
  app = createApp({ render: () => h(ApprovalDialog) })
    .use(pinia)
    .use(i18n);
  app.mount(document.createElement("div"));
  await nextTick();
  await nextTick();
  const dialog = document.body.querySelector<HTMLElement>("[data-mcp-approval]");
  if (!dialog) {
    throw new Error("the dialog did not open");
  }
  return dialog;
}

function button(where: HTMLElement, label: string): HTMLButtonElement {
  const found = [...where.querySelectorAll("button")].find((candidate) => candidate.textContent?.trim() === label);
  if (!found) {
    throw new Error(`the dialog has no ${label} button`);
  }
  return found;
}

beforeEach(() => {
  setActivePinia(createPinia());
  answerMock.mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  for (const node of document.querySelectorAll('[role="dialog"]')) {
    node.remove();
  }
});

describe("an external server's tool asking to act", () => {
  it("says which server and tool it came from, and that it was not written here", async () => {
    const dialog = await mount(external);

    // The reason the call stopped is the external one, and an absent origin is the MCP listener.
    expect(dialog.dataset.approvalReason).toBe("external");
    expect(dialog.dataset.approvalOrigin).toBe("mcp");

    expect(dialog.textContent).toContain("An external server's tool is asking to act");
    expect(dialog.textContent).toContain("files provides this tool");
    expect(dialog.textContent).toContain("read_file");
  });

  it("refuses through the answer command the rest of the dialog uses", async () => {
    const dialog = await mount(external);

    button(dialog, "Refuse").click();
    await vi.waitFor(() => expect(answerMock).toHaveBeenCalledWith("call-9", "denied"));
  });

  it("approves through the same command, with the other answer", async () => {
    const dialog = await mount(external);

    button(dialog, "Approve").click();
    await vi.waitFor(() => expect(answerMock).toHaveBeenCalledWith("call-9", "approved"));
  });
});

describe("a call this window's assistant asked for", () => {
  it("names the assistant and the risk that stopped it", async () => {
    const dialog = await mount(destructive);

    expect(dialog.dataset.approvalOrigin).toBe("ai");
    expect(dialog.dataset.approvalReason).toBe("risk");
    expect(dialog.textContent).toContain("The assistant in this window");
    expect(dialog.textContent).toContain("rm -rf /tmp/seven-shell-scratch");
  });
});
