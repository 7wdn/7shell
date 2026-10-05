import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  commandClassify: vi.fn(),
  batchRun: vi.fn(),
  knownHostTrust: vi.fn(),
  knownHostForget: vi.fn(),
}));
// The bus is mocked rather than subscribed: these tests are about what the store does with an
// event, and the bus has its own suite for receiving one.
vi.mock("@/lib/batch/bus", () => ({
  startBatchBus: vi.fn(async () => {}),
  onBatchProgress: vi.fn(() => () => {}),
  stopBatchBus: vi.fn(),
}));

import { batchRun, commandClassify } from "@/lib/backend/tauri";
import { onBatchProgress } from "@/lib/batch/bus";
import { useBatchStore, rowFailed } from "@/stores/batch";
import { useSessionsStore } from "@/stores/sessions";
import type { BatchHostResult, BatchOutcome, RiskAssessment } from "@/types/batch";
import type { Session } from "@/types/session";

const classifyMock = vi.mocked(commandClassify);
const runMock = vi.mocked(batchRun);
const progressMock = vi.mocked(onBatchProgress);

function session(id: string, name: string): Session {
  return {
    id,
    name,
    protocol: "ssh",
    host: "example.test",
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: true },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 0, forwardAgent: false },
    updatedAt: 0,
  };
}

function assessment(level: RiskAssessment["level"], ruleId = "recursive-force-delete"): RiskAssessment {
  return { level, reasons: [{ ruleId, labelKey: `risk.${ruleId}`, level }] };
}

function ran(sessionId: string, exitCode: number | null, stdout = ""): BatchHostResult {
  return {
    sessionId,
    sessionName: sessionId,
    outcome: { kind: "ran", exitCode, stdout, stderr: "" },
    durationMs: 12,
  };
}

function finished(results: BatchHostResult[], command = "uptime"): BatchOutcome {
  return { kind: "finished", command, risk: assessment("safe"), results, durationMs: 30 };
}

/** The progress listener the store registered, so a test can pretend to be the backend. */
function progressSink(): (progress: { kind: string; sessionId?: string; result?: BatchHostResult }) => void {
  const call = progressMock.mock.calls.at(-1);
  if (!call) {
    throw new Error("the store never registered a listener");
  }
  return call[0] as never;
}

describe("batch store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    classifyMock.mockResolvedValue(assessment("safe"));
    useSessionsStore().sessions = [session("one", "web-1"), session("two", "web-2")];
  });

  afterEach(() => {
    useBatchStore().forget();
  });

  it("asks the backend for a verdict as the command is typed, and shows its answer", async () => {
    const batch = useBatchStore();
    batch.listen();

    batch.setCommand("rm -rf /srv/app");
    classifyMock.mockResolvedValue(assessment("destructive"));

    await batch.grade();

    expect(classifyMock).toHaveBeenLastCalledWith("rm -rf /srv/app");
    expect(batch.risk?.level).toBe("destructive");
  });

  it("blocks a command that needs approval until somebody has read the reasons", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    classifyMock.mockResolvedValue(assessment("destructive"));

    batch.setCommand("rm -rf /srv/app");
    await batch.grade();

    expect(batch.blocked).toBe("approval");
    expect(batch.canRun).toBe(false);

    batch.acknowledge();
    expect(batch.canRun).toBe(true);
  });

  it("forgets the acknowledgement when the command changes", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();

    batch.setCommand("rm -rf /srv/app");
    await batch.grade();
    batch.acknowledge();
    expect(batch.canRun).toBe(true);

    batch.setCommand("rm -rf /srv/app/prod");
    expect(batch.acknowledged).toBe(false);
  });

  it("will not run with nothing selected, and will not run an empty command", () => {
    const batch = useBatchStore();
    batch.listen();

    batch.setCommand("uptime");
    expect(batch.blocked).toBe("no-hosts");

    batch.selectAll();
    expect(batch.canRun).toBe(true);

    batch.setCommand("   ");
    expect(batch.blocked).toBe("empty");
  });

  it("runs on the selected hosts in the library's order, however they were ticked", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.toggle("two");
    batch.toggle("one");
    batch.setCommand("uptime");
    runMock.mockResolvedValue(finished([ran("one", 0), ran("two", 0)]));

    await batch.run();

    expect(runMock).toHaveBeenCalledWith({
      sessionIds: ["one", "two"],
      command: "uptime",
      timeoutMs: 30_000,
      parallel: 4,
      acknowledged: false,
    });
  });

  it("fills rows in from progress as hosts answer, and not from their positions", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("uptime");

    let answer: (outcome: BatchOutcome) => void = () => {};
    runMock.mockImplementation(
      () =>
        new Promise<BatchOutcome>((resolve) => {
          answer = resolve;
        }),
    );

    const running = batch.run();
    // The second host answers first, which must land in the second row and nowhere else.
    progressSink()({ kind: "hostStarted", sessionId: "two" });
    progressSink()({ kind: "hostFinished", result: ran("two", 0, "up 3 days\n") });

    expect(batch.rows.map((row) => row.status)).toEqual(["pending", "done"]);
    expect(batch.rows[1]?.result?.outcome).toMatchObject({ exitCode: 0 });

    answer(finished([ran("one", 0), ran("two", 0)]));
    await running;
    expect(batch.rows.map((row) => row.status)).toEqual(["done", "done"]);
  });

  it("keeps a failure to reach one host as that host's row", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("uptime");
    runMock.mockResolvedValue(
      finished([
        ran("one", 0),
        {
          sessionId: "two",
          sessionName: "web-2",
          outcome: {
            kind: "failed",
            error: {
              code: "SSH-TIMEOUT",
              messageKey: "errors.ssh.timeout",
              messageParams: { seconds: "30" },
            },
          },
          durationMs: 30_000,
        },
      ]),
    );

    await batch.run();

    expect(batch.failedIds).toEqual(["two"]);
    expect(rowFailed(batch.rows[0] as never)).toBe(false);
  });

  it("counts a command that exited non-zero as one to retry", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("uptime");
    runMock.mockResolvedValue(finished([ran("one", 0), ran("two", 3)]));

    await batch.run();

    expect(batch.failedIds).toEqual(["two"]);
  });

  it("retries only the hosts that did not succeed, and says so to the backend", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("deploy.sh");
    runMock.mockResolvedValue(finished([ran("one", 0), ran("two", 1)]));
    await batch.run();

    runMock.mockResolvedValue(finished([ran("two", 0)]));
    await batch.retryFailed();

    expect(runMock).toHaveBeenLastCalledWith({
      sessionIds: ["two"],
      command: "deploy.sh",
      timeoutMs: 30_000,
      parallel: 4,
      acknowledged: true,
    });
    expect(batch.failedIds).toEqual([]);
  });

  it("holds the request a host key decision stopped, and runs exactly it again", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("uptime");
    runMock.mockResolvedValue({
      kind: "hostKeyUnknown",
      host: { sessionId: "two", sessionName: "web-2" },
      key: { host: "example.test", port: 22, keyType: "ssh-ed25519", publicKey: "AAAA", fingerprint: "SHA256:AA" },
    });

    await batch.run();

    // Nothing ran, so there are no rows and no verdict — only the question.
    expect(batch.rows).toEqual([]);
    expect(batch.outcome).toBeNull();

    runMock.mockResolvedValue(finished([ran("one", 0), ran("two", 0)]));
    await batch.resumeAfterDecision();

    expect(runMock).toHaveBeenCalledTimes(2);
    expect(runMock.mock.calls[1]?.[0]).toEqual(runMock.mock.calls[0]?.[0]);
  });

  it("asks for the approval path explicitly rather than weakening the verdict", async () => {
    const batch = useBatchStore();
    batch.listen();
    batch.selectAll();
    batch.setCommand("rm -rf /srv/app");
    classifyMock.mockResolvedValue(assessment("destructive"));
    await batch.grade();
    runMock.mockResolvedValue({
      kind: "approvalRequired",
      risk: assessment("destructive"),
    });

    await batch.run();
    expect(batch.approval?.level).toBe("destructive");
    expect(batch.rows).toEqual([]);

    runMock.mockResolvedValue(finished([ran("one", 0), ran("two", 0)]));
    await batch.runAcknowledged();

    expect(runMock.mock.calls[1]?.[0]?.acknowledged).toBe(true);
    expect(batch.rows).toHaveLength(2);
  });
});
