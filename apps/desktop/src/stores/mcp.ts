/**
 * The agent interface as the window watches it: whether it is on, what it has done, and what it is
 * waiting for a person to answer.
 *
 * Three things are kept apart on purpose:
 *
 * 1. **The listener's status and the settings file are two answers.** The status is what is actually
 *    bound to a port; the settings are what the next start will use. A page that read `mcpEnabled`
 *    out of the file would draw "on" over a listener that failed to start.
 * 2. **An approval is a question, not an event to log.** It stays in `pendingApproval` until it is
 *    answered, an answer fails to arrive, or its deadline passes — and only then does it leave. A
 *    request that vanished because a redraw happened is a request nobody answered.
 * 3. **The audit is read, never accumulated.** `loadAudit` asks for the tail and replaces what was
 *    on screen, so the table cannot drift from the file it is a window onto.
 *
 * The subscription is made once for the whole window: the question has to be drawable whatever tab
 * is on screen, which is also why the store is initialized by the frame rather than by the page that
 * configures it.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import type { Unlisten } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import {
  type McpApprovalDecision,
  type McpApprovalRequest,
  type McpAuditEntry,
  MCP_AUDIT_TAIL_LIMIT,
  type McpStatus,
} from "@/types/mcp";

export const useMcpStore = defineStore("mcp", () => {
  /** What the listener says it is doing, or nothing until it has been asked once. */
  const status = ref<McpStatus | null>(null);
  /** The tail of the audit file, newest last — the order it was written in. */
  const audit = ref<McpAuditEntry[]>([]);
  /** The call waiting for an answer, or nothing while none is. */
  const pendingApproval = ref<McpApprovalRequest | null>(null);
  /** The last failure, kept until the next attempt rather than shouted: the page draws it. */
  const failure = ref<ShellError | null>(null);
  /** Whether a start or a stop is in flight. */
  const changing = ref(false);
  /** Whether the audit has been asked for at least once, so an empty table can say "none yet". */
  const auditLoaded = ref(false);
  const loadingAudit = ref(false);
  /** Whether the last copy of the client configuration worked, for the button's own line. */
  const copyFailed = ref(false);
  const copying = ref(false);

  /** Whether anything is listening, read from the status rather than from the settings. */
  const enabled = computed(() => status.value?.enabled === true);

  /** Asks the listener what it is doing. A failure leaves the last answer on screen. */
  async function refresh(): Promise<void> {
    failure.value = null;
    try {
      status.value = await backend.mcp.status();
    } catch (error) {
      failure.value = asShellError(error, "mcp_status");
    }
  }

  /**
   * Starts the listener, or restarts it with what the settings now hold.
   *
   * One call for both because there is one state worth reporting: running with the configuration on
   * screen. The page saves first, so what it restarts into is what somebody just chose.
   */
  async function enable(): Promise<boolean> {
    if (changing.value) {
      return false;
    }
    changing.value = true;
    failure.value = null;
    try {
      status.value = await backend.mcp.enable();
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_enable");
      return false;
    } finally {
      changing.value = false;
    }
  }

  /** Stops it. The endpoint file goes with it, on the far side. */
  async function disable(): Promise<boolean> {
    if (changing.value) {
      return false;
    }
    changing.value = true;
    failure.value = null;
    try {
      status.value = await backend.mcp.disable();
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_disable");
      return false;
    } finally {
      changing.value = false;
    }
  }

  /**
   * Reads the last lines of the audit file.
   *
   * Replaces what was there rather than appending: the file is the record, and a table built by
   * appending would show a call twice after a refresh and miss ones that arrived while nobody asked.
   */
  async function loadAudit(limit: number = MCP_AUDIT_TAIL_LIMIT): Promise<void> {
    if (loadingAudit.value) {
      return;
    }
    loadingAudit.value = true;
    failure.value = null;
    try {
      audit.value = await backend.mcp.auditTail(limit);
    } catch (error) {
      failure.value = asShellError(error, "mcp_audit_tail");
    } finally {
      auditLoaded.value = true;
      loadingAudit.value = false;
    }
  }

  /**
   * Answers the call on screen.
   *
   * The dialog is cleared only once the answer has been delivered: a question that disappeared while
   * the answer was still in flight would look answered even if the listener never heard it. The
   * count the listener reports is refreshed afterwards, because "how many are still waiting" is its
   * number and not one this side can work out.
   */
  async function answerApproval(callId: string, decision: McpApprovalDecision): Promise<boolean> {
    failure.value = null;
    try {
      await backend.mcp.answerApproval(callId, decision);
    } catch (error) {
      failure.value = asShellError(error, "mcp_approval_answer");
      return false;
    }

    if (pendingApproval.value?.callId === callId) {
      pendingApproval.value = null;
    }
    void refresh();
    return true;
  }

  /**
   * Drops a question whose deadline has passed.
   *
   * Only used after the answer was refused *because* the deadline passed: the listener has already
   * stopped waiting, so a dialog that stayed up would be asking about a call that no longer exists —
   * and the next attempt to answer it would fail the same way.
   */
  function forgetApproval(callId: string): void {
    if (pendingApproval.value?.callId === callId) {
      pendingApproval.value = null;
    }
  }

  /**
   * Puts the client configuration on the clipboard.
   *
   * The text is copied and not drawn, and that is the whole point: it names a program and a file on
   * this machine, which is what a client's configuration needs and what nobody reads on a settings
   * page. Nothing secret is in it — the token stays in the endpoint file — and this is also why the
   * button is safe to press in front of somebody.
   */
  async function copyClientConfig(): Promise<boolean> {
    if (copying.value) {
      return false;
    }
    copying.value = true;
    failure.value = null;
    copyFailed.value = false;
    try {
      const text = await backend.mcp.clientConfig();
      await navigator.clipboard.writeText(text);
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_client_config");
      copyFailed.value = true;
      return false;
    } finally {
      copying.value = false;
    }
  }

  /**
   * Listens for the two things the listener says on its own.
   *
   * A question and a status change, each pushed rather than polled: the first has to reach a person
   * while the listener waits, and the second happens when the listener is started from a place this
   * window did not press. The returned function drops both, and a subscription that arrives after
   * the window has been torn down is dropped at once — a listener outliving the store would keep
   * asking a dead page to draw.
   */
  function initialize(): () => void {
    let stopped = false;
    let stopApproval: Unlisten | null = null;
    let stopStatus: Unlisten | null = null;

    void (async () => {
      try {
        const [approval, listener] = await Promise.all([
          backend.events.onMcpApproval((request) => {
            // A new question replaces the one on screen: the listener asks one at a time, and the
            // older question's deadline is the listener's to enforce.
            pendingApproval.value = request;
            failure.value = null;
          }),
          backend.events.onMcpStatus((next) => {
            status.value = next;
          }),
        ]);
        if (stopped) {
          approval();
          listener();
          return;
        }
        stopApproval = approval;
        stopStatus = listener;
      } catch (error) {
        failure.value = asShellError(error, "mcp_listen");
      }
    })();

    void refresh();

    return () => {
      stopped = true;
      stopApproval?.();
      stopStatus?.();
    };
  }

  return {
    status,
    audit,
    pendingApproval,
    failure,
    changing,
    auditLoaded,
    loadingAudit,
    copyFailed,
    copying,
    enabled,
    refresh,
    enable,
    disable,
    loadAudit,
    answerApproval,
    forgetApproval,
    copyClientConfig,
    initialize,
  };
});
