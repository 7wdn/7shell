/**
 * The agent interface as the window watches it: whether it is on, what it has done, what it is
 * waiting for a person to answer, and the external servers whose tools it may call.
 *
 * Four things are kept apart on purpose:
 *
 * 1. **The listener's status and the settings file are two answers.** The status is what is actually
 *    bound to a port; the settings are what the next start will use. A page that read `mcpEnabled`
 *    out of the file would draw "on" over a listener that failed to start.
 * 2. **An approval is a question, not an event to log.** It stays in `pendingApproval` until it is
 *    answered, an answer fails to arrive, or its deadline passes — and only then does it leave. A
 *    request that vanished because a redraw happened is a request nobody answered.
 * 3. **The audit is read, never accumulated.** `loadAudit` asks for the tail and replaces what was
 *    on screen, so the table cannot drift from the file it is a window onto.
 * 4. **A server's row is what the host last said, not what the form typed.** Its state and its
 *    secret state come from the answers and the events; the configuration is mirrored into the
 *    settings store only so that a save from another page cannot erase it.
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
import { usePreferencesStore } from "@/stores/preferences";
import {
  type McpApprovalDecision,
  type McpApprovalRequest,
  type McpAuditEntry,
  MCP_AUDIT_TAIL_LIMIT,
  type McpCatalog,
  type McpServerConfig,
  type McpServerStatus,
  type McpServerView,
  type McpStatus,
} from "@/types/mcp";

export const useMcpStore = defineStore("mcp", () => {
  const preferences = usePreferencesStore();

  /** What the listener says it is doing, or nothing until it has been asked once. */
  const status = ref<McpStatus | null>(null);
  /** The tail of the audit file, newest last — the order it was written in. */
  const audit = ref<McpAuditEntry[]>([]);
  /**
   * The external servers, each with its state and whether the vault holds secrets for it.
   *
   * The list comes from the host rather than from the settings file: what is configured and what
   * is running are two answers, and only the host knows the second. It is mirrored into the
   * preferences store as configuration alone, because the next settings save writes that list back.
   */
  const servers = ref<McpServerView[]>([]);
  /** Whether the server list has been read once, so an empty one can say "none configured". */
  const serversLoaded = ref(false);
  const loadingServers = ref(false);
  /** The server a write is in flight for: one row waits without freezing the rest of the page. */
  const busyServer = ref<string | null>(null);
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
  /**
   * The tool table the host built, or nothing until it has been asked for once.
   *
   * The registry is the authority on what an agent may call and on what is switched off, so the
   * capabilities page draws this answer rather than recomputing it from the settings file. It is
   * replaced whole on every read: a tool that disappeared from the catalog must not linger.
   */
  const catalog = ref<McpCatalog | null>(null);
  /** Whether the catalog has been read once, so an empty answer can say "none" rather than "reading". */
  const catalogLoaded = ref(false);
  const loadingCatalog = ref(false);

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
   * Replaces the list with the host's answer.
   *
   * Replaced rather than merged: the host is the authority on which servers exist, and a row kept
   * from an earlier read would be a server somebody already removed. A failure leaves the last
   * answer on screen, the same rule the listener's own status follows.
   */
  async function loadServers(): Promise<void> {
    if (loadingServers.value) {
      return;
    }
    loadingServers.value = true;
    failure.value = null;
    try {
      servers.value = await backend.mcp.servers();
      syncServerConfigs();
    } catch (error) {
      failure.value = asShellError(error, "mcp_servers_list");
    } finally {
      serversLoaded.value = true;
      loadingServers.value = false;
    }
  }

  /**
   * Reads the tool table the host builds.
   *
   * Replaced rather than merged, like the audit and the server list: the catalog is one answer to
   * "what may an agent call now", and a row kept from an earlier read would be a tool somebody just
   * switched off. A failure leaves the last answer on screen so the page keeps drawing something.
   */
  async function loadCatalog(): Promise<void> {
    if (loadingCatalog.value) {
      return;
    }
    loadingCatalog.value = true;
    failure.value = null;
    try {
      catalog.value = await backend.mcp.catalog();
    } catch (error) {
      failure.value = asShellError(error, "mcp_catalog");
    } finally {
      catalogLoaded.value = true;
      loadingCatalog.value = false;
    }
  }

  /**
   * Writes one server's configuration, and takes the row the host answered with.
   *
   * The answer is adopted rather than the draft: the host normalizes the id, trims what it stores
   * and decides where supervision starts, so what the page draws next is its answer and not the
   * form's guess.
   */
  async function saveServer(config: McpServerConfig): Promise<boolean> {
    if (busyServer.value !== null) {
      return false;
    }
    busyServer.value = config.id;
    failure.value = null;
    try {
      adoptServer(await backend.mcp.saveServer(config));
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_server_save");
      return false;
    } finally {
      busyServer.value = null;
    }
  }

  /** Removes one server, with the secrets the host files under it. */
  async function removeServer(serverId: string): Promise<boolean> {
    if (busyServer.value !== null) {
      return false;
    }
    busyServer.value = serverId;
    failure.value = null;
    try {
      await backend.mcp.removeServer(serverId);
      servers.value = servers.value.filter((view) => view.config.id !== serverId);
      syncServerConfigs();
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_server_remove");
      return false;
    } finally {
      busyServer.value = null;
    }
  }

  /**
   * Keeps the environment and headers one server is reached with.
   *
   * Empty objects clear what is stored, which is the host's own rule. Nothing is read back: the
   * answer carries `hasSecrets`, and `null` there means the vault is locked rather than "none".
   */
  async function setServerSecrets(
    serverId: string,
    env: Record<string, string>,
    headers: Record<string, string>,
  ): Promise<boolean> {
    if (busyServer.value !== null) {
      return false;
    }
    busyServer.value = serverId;
    failure.value = null;
    try {
      adoptServer(await backend.mcp.setServerSecrets(serverId, env, headers));
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_server_secrets_set");
      return false;
    } finally {
      busyServer.value = null;
    }
  }

  /** Reconnects one server now, and takes the state that attempt reached. */
  async function refreshServer(serverId: string): Promise<boolean> {
    if (busyServer.value !== null) {
      return false;
    }
    busyServer.value = serverId;
    failure.value = null;
    try {
      adoptServer(await backend.mcp.refreshServer(serverId));
      return true;
    } catch (error) {
      failure.value = asShellError(error, "mcp_server_refresh");
      return false;
    } finally {
      busyServer.value = null;
    }
  }

  /** Replaces one row, or appends it when the host answered about a server this side did not have. */
  function adoptServer(view: McpServerView): void {
    const known = servers.value.some((entry) => entry.config.id === view.config.id);
    servers.value = known
      ? servers.value.map((entry) => (entry.config.id === view.config.id ? view : entry))
      : [...servers.value, view];
    syncServerConfigs();
  }

  /**
   * Keeps the settings store's copy of the list in step with what the host just answered.
   *
   * Without this the next save from any other settings page would write the list it read at
   * startup — erasing a server that was added or renamed since.
   */
  function syncServerConfigs(): void {
    preferences.setMcpClientServers(servers.value.map((view) => view.config));
  }

  /**
   * Takes one status event.
   *
   * A server this side has no row for is not invented from the event: the event carries a state and
   * a tool count, not a configuration, and a row drawn from it would have nothing to edit. The list
   * is asked again instead, which is also how a server added by another window appears here.
   */
  function applyServerStatus(next: McpServerStatus): void {
    const known = servers.value.some((entry) => entry.config.id === next.serverId);
    if (!known) {
      void loadServers();
      return;
    }
    servers.value = servers.value.map((entry) =>
      entry.config.id === next.serverId ? { ...entry, status: next } : entry,
    );
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
   * Listens for the three things the host says on its own.
   *
   * A question, the listener's state and each external server's state, all pushed rather than
   * polled: the first has to reach a person while the listener waits, the second happens when the
   * listener is started from a place this window did not press, and the third changes on a server's
   * own timer — a row that only re-read when somebody looked would say "connecting" long after it
   * gave up. The returned function drops all three, and a subscription that arrives after the window
   * has been torn down is dropped at once — a listener outliving the store would keep asking a dead
   * page to draw.
   */
  function initialize(): () => void {
    let stopped = false;
    let stopApproval: Unlisten | null = null;
    let stopStatus: Unlisten | null = null;
    let stopServer: Unlisten | null = null;

    void (async () => {
      try {
        const [approval, listener, server] = await Promise.all([
          backend.events.onMcpApproval((request) => {
            // A new question replaces the one on screen: the listener asks one at a time, and the
            // older question's deadline is the listener's to enforce.
            pendingApproval.value = request;
            failure.value = null;
          }),
          backend.events.onMcpStatus((next) => {
            status.value = next;
          }),
          backend.events.onMcpServerStatus(applyServerStatus),
        ]);
        if (stopped) {
          approval();
          listener();
          server();
          return;
        }
        stopApproval = approval;
        stopStatus = listener;
        stopServer = server;
      } catch (error) {
        failure.value = asShellError(error, "mcp_listen");
      }
    })();

    void refresh();

    return () => {
      stopped = true;
      stopApproval?.();
      stopStatus?.();
      stopServer?.();
    };
  }

  return {
    status,
    audit,
    servers,
    serversLoaded,
    loadingServers,
    busyServer,
    pendingApproval,
    failure,
    changing,
    auditLoaded,
    loadingAudit,
    copyFailed,
    copying,
    catalog,
    catalogLoaded,
    loadingCatalog,
    enabled,
    refresh,
    enable,
    disable,
    loadAudit,
    loadCatalog,
    loadServers,
    saveServer,
    removeServer,
    setServerSecrets,
    refreshServer,
    answerApproval,
    forgetApproval,
    copyClientConfig,
    initialize,
  };
});
