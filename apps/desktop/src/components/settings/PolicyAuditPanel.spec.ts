/**
 * The policy audit page, as the window draws it.
 *
 * The trail is the backend's answer, so the test mocks that answer and asserts what the table does
 * with it: the newest decision stays last, every field the contract names has a column, and a
 * failure to read the file is said out loud rather than drawn as an empty trail.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  policyAuditTail: vi.fn(),
}));

import PolicyAuditPanel from "@/components/settings/PolicyAuditPanel.vue";
import { i18n } from "@/i18n";
import { policyAuditTail } from "@/lib/backend/tauri";
import type { PolicyAuditEvent } from "@/types/app";

const auditMock = vi.mocked(policyAuditTail);

let app: App | undefined;
let host: HTMLDivElement;

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(PolicyAuditPanel) }).use(i18n);
  app.mount(host);
  await vi.waitFor(() => expect(auditMock).toHaveBeenCalled());
  await nextTick();
}

function rows(): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>("[data-policy-audit-row]")];
}

/** Three decisions in the order the file holds them: oldest first, newest last. */
const TRAIL: PolicyAuditEvent[] = [
  {
    at: 1_760_000_000,
    action: "batch_run",
    risk: { level: "safe", reasons: [] },
    decision: "allow",
    source: "global",
    sessionId: "web-1",
    groupId: "prod",
    host: "web-1.example.test",
  },
  {
    at: 1_760_000_060,
    action: "container_action",
    risk: {
      level: "destructive",
      reasons: [{ ruleId: "container.stop", labelKey: "risk.container.stop", level: "destructive" }],
    },
    decision: "ask",
    source: "host:web-1.example.test:22",
    subject: "container.stop",
    host: "web-1.example.test",
  },
  {
    at: 1_760_000_120,
    action: "sftp_write_text",
    risk: { level: "caution", reasons: [] },
    decision: "deny",
    source: "directory:/srv",
    sessionId: "db-1",
    directory: "/srv/app/config",
  },
];

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the audit table", () => {
  it("draws every decision newest last, with the layer and target it looked at", async () => {
    auditMock.mockResolvedValue(TRAIL);
    await mount();
    await vi.waitFor(() => expect(rows()).toHaveLength(3));

    const [first, second, last] = rows();
    expect(first?.textContent).toContain("batch_run");
    expect(first?.textContent).toContain("Allow");
    expect(first?.textContent).toContain("Safe");
    expect(first?.textContent).toContain("global");
    expect(first?.textContent).toContain("Session: web-1");
    expect(first?.textContent).toContain("Group: prod");
    expect(first?.textContent).toContain("Host: web-1.example.test");

    expect(second?.textContent).toContain("container_action");
    expect(second?.textContent).toContain("Ask");
    expect(second?.textContent).toContain("Destructive");
    expect(second?.textContent).toContain("host:web-1.example.test:22");
    // The redacted subject the host kept travels under the operation's name.
    expect(second?.textContent).toContain("container.stop");

    expect(last?.textContent).toContain("sftp_write_text");
    expect(last?.textContent).toContain("Deny");
    expect(last?.textContent).toContain("Caution");
    expect(last?.textContent).toContain("directory:/srv");
    expect(last?.textContent).toContain("Directory: /srv/app/config");
  });

  it("says that the trail is empty rather than drawing a blank table", async () => {
    auditMock.mockResolvedValue([]);
    await mount();

    expect(rows()).toHaveLength(0);
    await vi.waitFor(() => expect(host.textContent).toContain("No decision has been recorded yet."));
  });

  it("reports a trail that could not be read, and can be asked to read it again", async () => {
    auditMock.mockRejectedValueOnce(new Error("the disk is gone")).mockResolvedValueOnce(TRAIL);
    await mount();

    await vi.waitFor(() => expect(host.querySelector("[data-policy-audit-problem]")).not.toBeNull());
    expect(host.textContent).toContain("The trail could not be read.");

    const refresh = host.querySelector<HTMLButtonElement>("[data-policy-audit-refresh]");
    if (!refresh) {
      throw new Error("the panel has no refresh button");
    }
    refresh.click();

    await vi.waitFor(() => expect(rows()).toHaveLength(3));
    expect(host.querySelector("[data-policy-audit-problem]")).toBeNull();
    expect(auditMock.mock.calls.length).toBeGreaterThanOrEqual(2);
  });
});
