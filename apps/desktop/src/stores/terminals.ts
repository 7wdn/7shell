/**
 * The tabs that are open — a terminal or a file browser each — and the host key decisions that
 * stand in front of them.
 *
 * The order `session_open`/`files_open` enforce is visible here rather than smoothed over: a tab is
 * only created once the backend has actually opened something. A host key that has to be decided
 * first produces no tab at all — there is nothing to draw, and drawing an empty one would suggest a
 * connection that does not exist. A *failed* open does produce a tab, because that is where the
 * person who asked for it is looking.
 *
 * A terminal that ends keeps its tab: the reason the server gave and a way to connect again are
 * both things only that tab can offer. A file browser has no output stream to end, so its tab is
 * either open or failed — and closing it goes through the backend, because that is what cancels
 * whatever the browser was still moving.
 *
 * There is one prompt for a host key, not one per feature: a decision does not care what was being
 * opened, so it only carries which open to repeat once it is answered.
 *
 * The forwards live here too, for the same reason the tabs do: `session_open` answers with them,
 * so this is the store that already holds the answer. Keeping them in one place is what lets the
 * sidebar draw them and stops the same failed forward from being reported once per component.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { encodeBase64, endedTerminal, onTerminalEnded, startTerminalBus } from "@/lib/terminal/bus";
import { useFilesStore } from "@/stores/files";
import type {
  KnownHostEntry,
  OpenOutcome,
  OpenTunnel,
  PresentedHostKey,
  Session,
  SftpOutcome,
  TunnelFailure,
} from "@/types/session";

export type TerminalTabStatus = "open" | "ended" | "failed";

/** A file browser has no stream that can end: it opened, or it did not. */
export type FilesTabStatus = "open" | "failed";

/** What was being opened when a host key decision interrupted it. */
export type OpenPurpose = "terminal" | "files";

/** The half of an open answer that is not an open: the decision that has to come first. */
type DecisionOutcome = Exclude<OpenOutcome | SftpOutcome, { kind: "opened" }>;

export interface TerminalTab {
  readonly kind: "terminal";
  id: string;
  readonly session: Session;
  /** The handle the backend knows this terminal by, once there is one. */
  handle: string | null;
  status: TerminalTabStatus;
  /** What the server said when the terminal ended, when it said anything. */
  reason?: string;
  /** The envelope of the last failure this tab is responsible for showing. */
  failure: ShellError | null;
}

/** A file browser, drawn as a tab. The session names it; the handle says which browser it is. */
export interface FilesTab {
  readonly kind: "files";
  id: string;
  readonly session: Session;
  handle: string | null;
  status: FilesTabStatus;
  failure: ShellError | null;
}

export type WorkspaceTab = TerminalTab | FilesTab;

/** A host key the backend is waiting on before it will send anything at all. */
export type HostKeyDecision =
  | {
      readonly kind: "hostKeyUnknown";
      readonly session: Session;
      /** Which open to repeat once this is answered. */
      readonly purpose: OpenPurpose;
      readonly key: PresentedHostKey;
    }
  | {
      readonly kind: "hostKeyMismatch";
      readonly session: Session;
      readonly purpose: OpenPurpose;
      readonly key: PresentedHostKey;
      readonly trusted: KnownHostEntry;
    };

/**
 * What tells one failed forward from another when it is dismissed. The same saved forward failing
 * twice replaces its old reason instead of adding a second line.
 */
export function tunnelFailureKey(failure: TunnelFailure): string {
  return `${failure.forwardId}@${failure.bindHost}:${failure.bindPort}`;
}

export const useTerminalsStore = defineStore("terminals", () => {
  const files = useFilesStore();
  const tabs = ref<WorkspaceTab[]>([]);
  const activeTabId = ref<string | null>(null);
  /** Session ids with an open request in flight, so a row can say so and a double-click cannot. */
  const connectingSessionIds = ref<string[]>([]);
  const decision = ref<HostKeyDecision | null>(null);
  /** The forwards that are up, as the backend last described them. */
  const tunnels = ref<OpenTunnel[]>([]);
  /**
   * Forwards that were asked for and did not come up. The open outcome carries them, and they stay
   * until dismissed: "the tunnel is not there" is exactly what answers "the tunnel is up but
   * nothing works", so throwing them away with the answer would lose the useful half.
   */
  const tunnelFailures = ref<TunnelFailure[]>([]);
  /** A stop, or a re-read of the forward list, that failed. Shown where the forwards are. */
  const tunnelProblem = ref<ShellError | null>(null);

  /** Subscribes to terminal events. Call once, from the shell. Returns the unsubscribe. */
  function initialize(): () => void {
    void startTerminalBus();
    // Forwards can already be up by the time this store exists, and the sidebar draws what the
    // backend says rather than what this store happens to have seen.
    void refreshTunnels();
    return onTerminalEnded(markEnded);
  }

  function markEnded(handle: string, reason?: string): void {
    // A terminal takes its forwards with it, whether or not a tab is drawing the handle.
    void refreshTunnels();
    const tab = tabs.value.find((entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === handle);
    if (!tab) {
      // No tab draws this handle. Anything that creates one later asks the bus what happened,
      // which is why the reason is remembered rather than dropped here.
      return;
    }
    tab.status = "ended";
    tab.reason = reason;
  }

  /** Opens a terminal for a saved session, or raises the decision that has to come first. */
  async function open(session: Session): Promise<void> {
    if (connectingSessionIds.value.includes(session.id)) {
      return;
    }
    connectingSessionIds.value.push(session.id);
    try {
      acceptTerminal(session, await backend.terminal.open(session.id));
    } catch (error) {
      addTerminal(session, { handle: null, status: "failed", failure: asShellError(error, "session_open") });
    } finally {
      connectingSessionIds.value = connectingSessionIds.value.filter((id) => id !== session.id);
    }
  }

  /**
   * Opens the file browser for a saved session.
   *
   * One browser per session: the backend would happily open a second one over the same connection,
   * and two tabs showing the same directory would then drift apart with no way to tell which is
   * which. A second ask focuses the tab that exists.
   */
  async function openFiles(session: Session): Promise<void> {
    const existing = tabs.value.find(
      (entry): entry is FilesTab => entry.kind === "files" && entry.session.id === session.id,
    );
    if (existing && existing.status === "open") {
      activeTabId.value = existing.id;
      return;
    }
    if (connectingSessionIds.value.includes(session.id)) {
      return;
    }
    // A tab whose open failed holds no browser at all: asking again is a retry, not a second view.
    if (existing) {
      forget(existing.id);
    }

    connectingSessionIds.value.push(session.id);
    try {
      acceptFiles(session, await files.open(session.id));
    } catch (error) {
      addFiles(session, { handle: null, status: "failed", failure: asShellError(error, "files_open") });
    } finally {
      connectingSessionIds.value = connectingSessionIds.value.filter((id) => id !== session.id);
    }
  }

  function acceptTerminal(session: Session, outcome: OpenOutcome): void {
    if (outcome.kind === "opened") {
      rememberTunnels(outcome.tunnels);
      rememberTunnelFailures(outcome.tunnelFailures);
      // A terminal that died before its tab existed is shown as ended, with the reason it gave.
      const ended = endedTerminal(outcome.handle);
      addTab({
        kind: "terminal",
        id: crypto.randomUUID(),
        session,
        handle: outcome.handle,
        status: ended ? "ended" : "open",
        reason: ended?.reason,
        failure: null,
      });
      return;
    }

    raiseDecision(session, "terminal", outcome);
  }

  function acceptFiles(session: Session, outcome: SftpOutcome): void {
    if (outcome.kind === "opened") {
      addTab({
        kind: "files",
        id: crypto.randomUUID(),
        session,
        handle: outcome.handle,
        status: "open",
        failure: null,
      });
      return;
    }

    raiseDecision(session, "files", outcome);
  }

  /** The one place a decision is raised, whatever was being opened when it came up. */
  function raiseDecision(session: Session, purpose: OpenPurpose, outcome: DecisionOutcome): void {
    decision.value =
      outcome.kind === "hostKeyMismatch"
        ? { kind: "hostKeyMismatch", session, purpose, key: outcome.key, trusted: outcome.trusted }
        : { kind: "hostKeyUnknown", session, purpose, key: outcome.key };
  }

  function addTab(tab: WorkspaceTab): void {
    tabs.value.push(tab);
    activeTabId.value = tab.id;
  }

  function addTerminal(
    session: Session,
    fields: { handle: string | null; status: TerminalTabStatus; reason?: string; failure: ShellError | null },
  ): void {
    addTab({ kind: "terminal", id: crypto.randomUUID(), session, ...fields });
  }

  function addFiles(
    session: Session,
    fields: { handle: string | null; status: FilesTabStatus; failure: ShellError | null },
  ): void {
    addTab({ kind: "files", id: crypto.randomUUID(), session, ...fields });
  }

  /** Records the key, then asks to open again — the decision itself opens nothing. */
  async function confirmTrust(): Promise<void> {
    const pending = decision.value;
    if (!pending || pending.kind !== "hostKeyUnknown") {
      return;
    }
    decision.value = null;
    await trustThenOpen(pending.session, pending.key, pending.purpose);
  }

  /** The mismatch path: forget the trusted key, record the new one, then open. */
  async function confirmReplace(): Promise<void> {
    const pending = decision.value;
    if (!pending || pending.kind !== "hostKeyMismatch") {
      return;
    }
    decision.value = null;
    try {
      await backend.knownHosts.forget(pending.key.host, pending.key.port);
    } catch (error) {
      failedOpen(pending.session, pending.purpose, asShellError(error, "known_host_forget"));
      return;
    }
    await trustThenOpen(pending.session, pending.key, pending.purpose);
  }

  async function trustThenOpen(session: Session, key: PresentedHostKey, purpose: OpenPurpose): Promise<void> {
    try {
      await backend.knownHosts.trust(key);
    } catch (error) {
      failedOpen(session, purpose, asShellError(error, "known_host_trust"));
      return;
    }
    if (purpose === "files") {
      await openFiles(session);
      return;
    }
    await open(session);
  }

  /** A failure before a tab existed still gets a tab: that is where the person is looking. */
  function failedOpen(session: Session, purpose: OpenPurpose, failure: ShellError): void {
    if (purpose === "files") {
      addFiles(session, { handle: null, status: "failed", failure });
      return;
    }
    addTerminal(session, { handle: null, status: "failed", failure });
  }

  /** Answers "cancel": nothing was sent, and nothing is left pending. */
  function dismiss(): void {
    decision.value = null;
  }

  async function closeTab(id: string): Promise<void> {
    const tab = tabs.value.find((entry) => entry.id === id);
    if (!tab) {
      return;
    }
    if (tab.kind === "files") {
      await closeFilesTab(tab);
      return;
    }

    // A tab whose terminal already ended, or whose close already failed, has nothing left to
    // close: removing it locally is the whole operation.
    if (!tab.handle || tab.status !== "open") {
      forget(id);
      await refreshTunnels();
      return;
    }
    try {
      await backend.terminal.close(tab.handle);
      forget(id);
      // Closing a terminal closes the forwards that were carrying its traffic.
      await refreshTunnels();
    } catch (error) {
      tab.status = "failed";
      tab.failure = asShellError(error, "terminal_close");
    }
  }

  /**
   * Closes a file browser: the backend first, because closing is what cancels the transfers the
   * browser still had running, and the tab only afterwards. A browser the backend refused to close
   * is still open, so its tab stays and says why.
   */
  async function closeFilesTab(tab: FilesTab): Promise<void> {
    if (!tab.handle || tab.status !== "open") {
      forget(tab.id);
      return;
    }
    const failure = await files.close(tab.handle);
    if (failure) {
      tab.status = "failed";
      tab.failure = failure;
      return;
    }
    forget(tab.id);
  }

  /**
   * Re-reads the forwards that are up. Called when the answer could have changed behind this
   * store: a terminal was opened, a forward was stopped, a terminal ended.
   */
  async function refreshTunnels(): Promise<void> {
    try {
      tunnels.value = await backend.terminal.tunnels();
    } catch (error) {
      tunnelProblem.value = asShellError(error, "tunnels_open");
    }
  }

  /** Stops one forward. The backend answers only after the listener is gone, so the row goes too. */
  async function stopTunnel(id: string): Promise<void> {
    tunnelProblem.value = null;
    try {
      await backend.terminal.closeTunnel(id);
      tunnels.value = tunnels.value.filter((entry) => entry.id !== id);
    } catch (error) {
      tunnelProblem.value = asShellError(error, "tunnel_close");
    }
  }

  /** The open answer is the freshest word on what came up, but other terminals' forwards are
   *  still up and still theirs to show: merging is what keeps both. */
  function rememberTunnels(open: readonly OpenTunnel[]): void {
    const byId = new Map(tunnels.value.map((entry) => [entry.id, entry]));
    for (const tunnel of open) {
      byId.set(tunnel.id, tunnel);
    }
    tunnels.value = [...byId.values()];
  }

  function rememberTunnelFailures(failures: readonly TunnelFailure[]): void {
    if (failures.length === 0) {
      return;
    }
    const answering = new Set(failures.map(tunnelFailureKey));
    tunnelFailures.value = [
      ...tunnelFailures.value.filter((entry) => !answering.has(tunnelFailureKey(entry))),
      ...failures,
    ];
  }

  function dismissTunnelFailure(key: string): void {
    tunnelFailures.value = tunnelFailures.value.filter((entry) => tunnelFailureKey(entry) !== key);
  }

  function dismissTunnelProblem(): void {
    tunnelProblem.value = null;
  }

  async function reconnect(id: string): Promise<void> {
    const tab = tabs.value.find((entry) => entry.id === id);
    if (!tab) {
      return;
    }
    const { session } = tab;
    const purpose: OpenPurpose = tab.kind === "files" ? "files" : "terminal";
    forget(id);
    if (purpose === "files") {
      await openFiles(session);
      return;
    }
    await open(session);
  }

  function forget(id: string): void {
    tabs.value = tabs.value.filter((entry) => entry.id !== id);
    if (activeTabId.value === id) {
      activeTabId.value = tabs.value[tabs.value.length - 1]?.id ?? null;
    }
  }

  function activate(id: string): void {
    activeTabId.value = id;
  }

  /** Keystrokes and pastes, in the bytes the terminal produced. */
  async function send(handle: string, bytes: Uint8Array): Promise<void> {
    const tab = tabs.value.find((entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === handle);
    if (!tab || tab.status !== "open") {
      return;
    }
    try {
      await backend.terminal.write(handle, encodeBase64(bytes));
    } catch (error) {
      tab.failure = asShellError(error, "terminal_write");
    }
  }

  async function resize(handle: string, columns: number, rows: number): Promise<void> {
    const tab = tabs.value.find((entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === handle);
    if (!tab || tab.status !== "open") {
      return;
    }
    try {
      await backend.terminal.resize(handle, columns, rows);
    } catch (error) {
      tab.failure = asShellError(error, "terminal_resize");
    }
  }

  const activeTab = computed(() => tabs.value.find((entry) => entry.id === activeTabId.value) ?? null);

  return {
    tabs,
    activeTab,
    activeTabId,
    connectingSessionIds,
    decision,
    tunnels,
    tunnelFailures,
    tunnelProblem,
    initialize,
    open,
    openFiles,
    confirmTrust,
    confirmReplace,
    dismiss,
    closeTab,
    reconnect,
    activate,
    send,
    resize,
    refreshTunnels,
    stopTunnel,
    dismissTunnelFailure,
    dismissTunnelProblem,
  };
});
