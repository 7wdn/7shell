/**
 * The batch panel: which hosts, one command, and one row per host.
 *
 * Four rules shape what is here, and each of them comes from something that would otherwise be a
 * lie on screen:
 *
 * 1. **A row belongs to a host, not to a position.** Results arrive in the request's order, but the
 *    rows are keyed by session id, so a host that answered while another was still connecting lands
 *    in its own row rather than in the next free one.
 * 2. **The panel's verdict is the backend's verdict.** The level shown before pressing anything
 *    comes from the same classifier the run consults, so a command that is gated cannot look safe
 *    on screen; and an acknowledgement is forgotten the moment the command changes, because it was
 *    an answer about *that* command.
 * 3. **The answer is authoritative.** Progress events fill rows in as they land, and the finished
 *    run overwrites every row from its own results — so an event lost to a slow listener leaves no
 *    row half-filled.
 * 4. **A decision is not a failure.** A host key that needs answering comes back as a decision,
 *    nothing has run, and the request is kept so the answer can re-run it rather than the person
 *    having to remember what they typed.
 */

import { computed, ref } from "vue";
import { defineStore } from "pinia";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import { onBatchProgress, startBatchBus } from "@/lib/batch/bus";
import { needsApproval } from "@/lib/batch/risk";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { BatchHostResult, BatchOutcome, BatchRequest, RiskAssessment } from "@/types/batch";
import type { Session } from "@/types/session";

/** How long one host may take, as the panel offers it. */
export const DEFAULT_TIMEOUT_MS = 30_000;
/** How many hosts are worked on at once, as the panel offers it. */
export const DEFAULT_PARALLEL = 4;

/** One host's row: what is known about it before it answers, and what it answered. */
export interface BatchRow {
  readonly sessionId: string;
  readonly sessionName: string;
  readonly session: Session;
  /** `pending` until its turn comes, `running` while it is its turn, `done` once it answered. */
  status: "pending" | "running" | "done";
  result: BatchHostResult | null;
}

/** Whether a row's command did not succeed: it never ran, or it exited non-zero. */
export function rowFailed(row: BatchRow): boolean {
  const outcome = row.result?.outcome;
  if (!outcome) {
    return false;
  }
  return outcome.kind === "failed" || outcome.exitCode !== 0;
}

export const useBatchStore = defineStore("batch", () => {
  const library = useSessionsStore();

  /** The command being composed, and what the classifier makes of it. */
  const command = ref("");
  const risk = ref<RiskAssessment | null>(null);
  const grading = ref(false);

  /**
   * Whether somebody has said they have seen the reasons.
   *
   * Cleared whenever the command changes: the acknowledgement is about one command line, and
   * carrying it to the next one would be exactly the rubber-stamp this is meant to prevent.
   */
  const acknowledged = ref(false);

  /** The hosts ticked in the picker, by session id. */
  const selected = ref<ReadonlySet<string>>(new Set());

  const timeoutMs = ref(DEFAULT_TIMEOUT_MS);
  const parallel = ref(DEFAULT_PARALLEL);

  const running = ref(false);
  const rows = ref<BatchRow[]>([]);
  /** The last answer: a finished run, a verdict to confirm, or nothing yet. */
  const outcome = ref<BatchOutcome | null>(null);
  const failure = ref<ShellError | null>(null);
  /** The request of the last attempt, so a host key answer can repeat exactly it. */
  const lastRequest = ref<BatchRequest | null>(null);

  /** A sequence number, so a slow classify cannot overwrite the answer to a newer keystroke. */
  let gradeSequence = 0;
  let stopProgress: (() => void) | null = null;

  const approval = computed(() => (outcome.value?.kind === "approvalRequired" ? outcome.value.risk : null));
  const finished = computed(() => (outcome.value?.kind === "finished" ? outcome.value : null));

  /** Whether the command may be run right now, and why not when it may not. */
  const blocked = computed<"empty" | "no-hosts" | "approval" | "running" | null>(() => {
    if (running.value) {
      return "running";
    }
    if (command.value.trim() === "") {
      return "empty";
    }
    if (selected.value.size === 0) {
      return "no-hosts";
    }
    if (risk.value && needsApproval(risk.value.level) && !acknowledged.value) {
      return "approval";
    }
    return null;
  });

  const canRun = computed(() => blocked.value === null);

  /** The hosts whose row did not succeed, in the order they were asked for. */
  const failedIds = computed(() => rows.value.filter(rowFailed).map((row) => row.sessionId));

  /** Keeps the subscription for the life of the application, not of the panel. */
  function listen(): void {
    void startBatchBus();
    stopProgress ??= onBatchProgress(receive);
  }

  function setCommand(value: string): void {
    command.value = value;
    acknowledged.value = false;
    void grade();
  }

  /** Asks the backend what it makes of the command. Runs nothing, and costs no network. */
  async function grade(): Promise<void> {
    const asking = command.value;
    if (asking.trim() === "") {
      risk.value = null;
      return;
    }

    const sequence = ++gradeSequence;
    grading.value = true;
    try {
      const answer = await backend.commands.classify(asking);
      if (sequence === gradeSequence) {
        risk.value = answer;
      }
    } catch (error) {
      if (sequence === gradeSequence) {
        risk.value = null;
        failure.value = asShellError(error, "command_classify");
      }
    } finally {
      if (sequence === gradeSequence) {
        grading.value = false;
      }
    }
  }

  function toggle(sessionId: string): void {
    const next = new Set(selected.value);
    if (next.has(sessionId)) {
      next.delete(sessionId);
    } else {
      next.add(sessionId);
    }
    selected.value = next;
  }

  function selectAll(): void {
    selected.value = new Set(library.sessions.map((session) => session.id));
  }

  function clearSelection(): void {
    selected.value = new Set();
  }

  function acknowledge(): void {
    acknowledged.value = true;
  }

  function dismiss(): void {
    outcome.value = null;
    failure.value = null;
  }

  /**
   * A failure that happened while answering a decision — trusting a key that the vault refused,
   * for instance. It belongs here rather than in a terminal tab: the run it stopped was about
   * several hosts, and the panel is where its rows were about to be.
   */
  function reportFailure(error: ShellError): void {
    failure.value = error;
    rows.value = [];
  }

  /** Runs the composed command on the ticked hosts. */
  async function run(): Promise<void> {
    await execute(requestFor(selected.value, command.value, acknowledged.value));
  }

  /** The approval path: the same command, now that somebody has seen the reasons. */
  async function runAcknowledged(): Promise<void> {
    await execute(requestFor(selected.value, command.value, true));
  }

  /** Repeats one host's run: an ops answer to "this one may work now". */
  async function retryOne(sessionId: string): Promise<void> {
    await execute(requestFor(new Set([sessionId]), command.value, true));
  }

  /** Repeats the run for every host whose command did not succeed. */
  async function retryFailed(): Promise<void> {
    const ids = failedIds.value;
    if (ids.length === 0) {
      return;
    }
    await execute(requestFor(new Set(ids), command.value, true));
  }

  /** Repeats the last attempt exactly, which is what a host key answer asks for. */
  async function resumeAfterDecision(): Promise<void> {
    const request = lastRequest.value;
    if (!request) {
      return;
    }
    await execute(request);
  }

  async function execute(request: BatchRequest): Promise<void> {
    running.value = true;
    failure.value = null;
    rows.value = rowsFor(request.sessionIds);
    lastRequest.value = request;

    // The verdict of what is being run, not of what is in the box: a run resumed from a decision
    // is the same command, but the box may have been edited since — and the rows are what is true.
    if (request.command === command.value) {
      outcome.value = null;
    }

    try {
      const answer = await backend.batch.run(request);
      apply(answer);
    } catch (error) {
      failure.value = asShellError(error, "batch_run");
      rows.value = [];
    } finally {
      running.value = false;
    }
  }

  function apply(answer: BatchOutcome): void {
    if (answer.kind === "finished") {
      const byId = new Map(answer.results.map((result) => [result.sessionId, result]));
      rows.value = rows.value.map((row) => {
        const result = byId.get(row.sessionId);
        return result ? { ...row, status: "done", result } : row;
      });
      outcome.value = answer;
      return;
    }

    if (answer.kind === "approvalRequired") {
      rows.value = [];
      outcome.value = answer;
      return;
    }

    // A host key decision: nothing ran on any host, and the answer belongs to the shared prompt —
    // one question for the whole application, whichever feature raised it.
    rows.value = [];
    const session = library.sessions.find((entry) => entry.id === answer.host.sessionId);
    if (session) {
      useTerminalsStore().raiseBatchDecision(session, answer);
    }
  }

  function receive(progress: { kind: string; sessionId?: string; result?: BatchHostResult }): void {
    if (progress.kind === "hostStarted" && progress.sessionId) {
      update(progress.sessionId, (row) => ({ ...row, status: "running" }));
      return;
    }
    if (progress.kind === "hostFinished" && progress.result) {
      const result = progress.result;
      update(result.sessionId, (row) => ({ ...row, status: "done", result }));
    }
  }

  function update(sessionId: string, change: (row: BatchRow) => BatchRow): void {
    rows.value = rows.value.map((row) => (row.sessionId === sessionId ? change(row) : row));
  }

  /** Rows for a selection, in the library's order so a run's results read like the sidebar. */
  function rowsFor(sessionIds: readonly string[]): BatchRow[] {
    const wanted = new Set(sessionIds);
    return library.sessions
      .filter((session) => wanted.has(session.id))
      .map((session) => ({
        sessionId: session.id,
        sessionName: session.name,
        session,
        status: "pending" as const,
        result: null,
      }));
  }

  function requestFor(ids: ReadonlySet<string>, text: string, confirmed: boolean): BatchRequest {
    // The order is the library's rather than the reader's: a result table that reorders itself
    // between runs is a table nobody can compare two runs in.
    const sessionIds = library.sessions.filter((session) => ids.has(session.id)).map((session) => session.id);
    return {
      sessionIds,
      command: text,
      timeoutMs: timeoutMs.value,
      parallel: parallel.value,
      acknowledged: confirmed,
    };
  }

  function forget(): void {
    stopProgress?.();
    stopProgress = null;
  }

  return {
    command,
    risk,
    grading,
    acknowledged,
    selected,
    timeoutMs,
    parallel,
    running,
    rows,
    outcome,
    failure,
    approval,
    finished,
    blocked,
    canRun,
    failedIds,
    listen,
    setCommand,
    grade,
    toggle,
    selectAll,
    clearSelection,
    acknowledge,
    dismiss,
    reportFailure,
    run,
    runAcknowledged,
    retryOne,
    retryFailed,
    resumeAfterDecision,
    forget,
  };
});
