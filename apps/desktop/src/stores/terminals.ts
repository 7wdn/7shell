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
 *
 * A terminal tab can carry a file panel below it. That panel draws the *same* browser a file tab
 * would have drawn — one browser per session, moved rather than duplicated, because two listings
 * of one directory have no way to tell which of them is right. The panel also shares the tab's
 * fate: its SFTP channel rides on the connection the terminal opened, so the browser is closed
 * before the terminal is, and the other order would cut the channel out from under it.
 *
 * Whether a session opens as one half or two is a setting rather than something to ask for each
 * time — that is how somebody's work looks, not a decision per connection. Nothing here is fixed by
 * it: the panel is a toggle away on the terminal's own strip, and closing it costs one click.
 *
 * A third kind of tab is neither a terminal nor a browser: the monitor's pages, which are about a
 * host rather than connected to it. They are opened from the monitor and hold a session rather than
 * a handle, so they survive a reconnect and are drawn as "not connected" when the session has no
 * terminal at all — see [`InspectTab`].
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { encodeBase64, endedTerminal, onTerminalEnded, startTerminalBus } from "@/lib/terminal/bus";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
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

/**
 * What is to be opened once a host key decision has been answered.
 *
 * A decision does not care what it interrupted, but the answer has to go back where the question
 * came from. A panel is the one case that cannot be found again from its session — a session has
 * at most one, and the terminal tab it hangs under is the only thing that says which.
 */
export type OpenTarget =
  | { readonly to: "terminal" }
  | { readonly to: "files" }
  | { readonly to: "panel"; readonly tabId: string };

/** The half of an open answer that is not an open: the decision that has to come first. */
type DecisionOutcome = Exclude<OpenOutcome | SftpOutcome, { kind: "opened" }>;

/**
 * The file panel under a terminal.
 *
 * A `handle` is a browser — there is one, or there is not — and the status says how it is doing:
 * opening, open, or failed with the envelope that says why. A panel that failed to *close* keeps
 * its browser and its handle, and shows the reason in the footer under it.
 */
export interface TerminalPanel {
  handle: string | null;
  status: "opening" | "open" | "failed";
  failure: ShellError | null;
}

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
  /** The file panel below this terminal, or null when it is not shown. */
  panel: TerminalPanel | null;
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

/** The host pages the monitor opens: a closer look at one machine, in a tab of its own. */
export type InspectView = "system" | "processes" | "network";

/**
 * A page about a host rather than a thing connected to it: the system, the processes, the network.
 *
 * It holds no handle on purpose. A page is about a *session*, and reads through whatever terminal
 * that session has open at the moment — so a terminal that reconnects does not leave the page
 * pointing at a connection that no longer exists, and a session with no terminal open is drawn as
 * "not connected" rather than as a dead page.
 */
export interface InspectTab {
  readonly kind: "inspect";
  readonly view: InspectView;
  id: string;
  readonly session: Session;
}

export type WorkspaceTab = TerminalTab | FilesTab | InspectTab;

/** A host key the backend is waiting on before it will send anything at all. */
export type HostKeyDecision =
  | {
      readonly kind: "hostKeyUnknown";
      readonly session: Session;
      /** Which open to repeat once this is answered. */
      readonly target: OpenTarget;
      readonly key: PresentedHostKey;
    }
  | {
      readonly kind: "hostKeyMismatch";
      readonly session: Session;
      readonly target: OpenTarget;
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
  const preferences = usePreferencesStore();
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

  /**
   * Opens a terminal for a saved session, in the shape the settings ask for, or raises the decision
   * that has to come first.
   *
   * The file panel is opened onto the tab that exists once the connection is up, rather than onto
   * the request: a refused connection, a host key decision still to be answered and a terminal that
   * died on arrival all end with a tab that is not open, and none of those is a place for a browser.
   */
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

    if (preferences.sessionLayout !== "split") {
      return;
    }
    // The tab that was just asked for, in case this session already had one: the newest is the one
    // the person is looking at.
    const tab = [...tabs.value]
      .reverse()
      .find(
        (entry): entry is TerminalTab =>
          entry.kind === "terminal" && entry.session.id === session.id && entry.status === "open",
      );
    if (tab) {
      await openPanel(tab);
    }
  }

  /**
   * Opens the file browser for a saved session.
   *
   * One browser per session: the backend would happily open a second one over the same connection,
   * and two listings of the same directory would then drift apart with no way to tell which is
   * which. A second ask goes to whatever is drawing the browser that exists — a file tab, or the
   * panel of the terminal that is carrying it.
   */
  async function openFiles(session: Session): Promise<void> {
    const showing = tabs.value.find((entry) => entry.session.id === session.id && browserHandleOf(entry) !== null);
    if (showing) {
      activeTabId.value = showing.id;
      return;
    }
    if (connectingSessionIds.value.includes(session.id)) {
      return;
    }
    // A tab whose open failed holds no browser at all: asking again is a retry, not a second view.
    const failed = tabs.value.find(
      (entry): entry is FilesTab => entry.kind === "files" && entry.session.id === session.id,
    );
    if (failed) {
      forget(failed.id);
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
        panel: null,
      });
      return;
    }

    raiseDecision(session, { to: "terminal" }, outcome);
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

    raiseDecision(session, { to: "files" }, outcome);
  }

  /**
   * Opens one of the monitor's pages for a session, or goes to the one that is already open.
   *
   * One page per view per host: asking for the system page of a machine twice should take somebody
   * to the page they have, not stack a second copy that shows the same numbers a moment apart. The
   * page reads through whichever terminal the session has open, so the same page keeps working
   * across a reconnect — which is also why nothing here waits for a connection to exist.
   */
  function openInspect(view: InspectView, session: Session): void {
    const showing = tabs.value.find(
      (entry): entry is InspectTab =>
        entry.kind === "inspect" && entry.view === view && entry.session.id === session.id,
    );
    if (showing) {
      activeTabId.value = showing.id;
      return;
    }

    addTab({ kind: "inspect", view, id: crypto.randomUUID(), session });
  }

  /** The panel's answer to an open: a browser, or the decision that has to come first. */
  function acceptPanel(tab: TerminalTab, outcome: SftpOutcome): void {
    if (outcome.kind === "opened") {
      tab.panel = { handle: outcome.handle, status: "open", failure: null };
      return;
    }

    // A decision is not a browser, so there is nothing to draw while it is up — and a panel left
    // spinning would go on spinning if it were cancelled. Answering it asks for the panel again.
    tab.panel = null;
    raiseDecision(tab.session, { to: "panel", tabId: tab.id }, outcome);
  }

  /** The one place a decision is raised, whatever was being opened when it came up. */
  function raiseDecision(session: Session, target: OpenTarget, outcome: DecisionOutcome): void {
    decision.value =
      outcome.kind === "hostKeyMismatch"
        ? { kind: "hostKeyMismatch", session, target, key: outcome.key, trusted: outcome.trusted }
        : { kind: "hostKeyUnknown", session, target, key: outcome.key };
  }

  function addTab(tab: WorkspaceTab): void {
    tabs.value.push(tab);
    activeTabId.value = tab.id;
  }

  function addTerminal(
    session: Session,
    fields: { handle: string | null; status: TerminalTabStatus; reason?: string; failure: ShellError | null },
  ): void {
    addTab({ kind: "terminal", id: crypto.randomUUID(), session, panel: null, ...fields });
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
    await trustThenOpen(pending.session, pending.key, pending.target);
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
      failedOpen(pending.session, pending.target, asShellError(error, "known_host_forget"));
      return;
    }
    await trustThenOpen(pending.session, pending.key, pending.target);
  }

  async function trustThenOpen(session: Session, key: PresentedHostKey, target: OpenTarget): Promise<void> {
    try {
      await backend.knownHosts.trust(key);
    } catch (error) {
      failedOpen(session, target, asShellError(error, "known_host_trust"));
      return;
    }
    await openFor(session, target);
  }

  /** Opens whatever a host key decision was standing in front of. */
  async function openFor(session: Session, target: OpenTarget): Promise<void> {
    if (target.to === "files") {
      await openFiles(session);
      return;
    }
    if (target.to === "panel") {
      // The tab may be gone: a decision does not hold a tab open, and the person may have closed
      // it rather than answer.
      const tab = terminalTab(target.tabId);
      if (tab) {
        await openPanel(tab);
      }
      return;
    }
    await open(session);
  }

  /**
   * A failure before a tab existed still gets a tab: that is where the person is looking.
   *
   * A panel is the exception, because it never had a tab of its own: it belongs to a terminal that
   * already exists, and if that tab is gone there is nowhere left for the reason to be true.
   */
  function failedOpen(session: Session, target: OpenTarget, failure: ShellError): void {
    if (target.to === "files") {
      addFiles(session, { handle: null, status: "failed", failure });
      return;
    }
    if (target.to === "panel") {
      const tab = terminalTab(target.tabId);
      if (tab) {
        tab.panel = { handle: null, status: "failed", failure };
      }
      return;
    }
    addTerminal(session, { handle: null, status: "failed", failure });
  }

  function terminalTab(id: string): TerminalTab | undefined {
    return tabs.value.find((entry): entry is TerminalTab => entry.kind === "terminal" && entry.id === id);
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
    // A page holds nothing to close: what it was reading belongs to a terminal, and that terminal
    // decides for itself how long it lives.
    if (tab.kind === "inspect") {
      forget(id);
      return;
    }

    // A tab whose terminal already ended, or whose close already failed, has nothing left to
    // close: removing it locally is the whole operation.
    if (!tab.handle || tab.status !== "open") {
      releasePanel(tab);
      forget(id);
      await refreshTunnels();
      return;
    }

    // The panel goes first, and not only as tidiness: its SFTP channel rides on this terminal's
    // connection, so closing the terminal first would cut the channel out from under a browser
    // that is still moving files.
    if (!(await closePanel(tab))) {
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
   * Opens the file panel under a terminal.
   *
   * A session has one browser, so if a file tab is already drawing it, that browser moves here and
   * the tab goes. What the person asked for is the files *under this terminal*, not a second copy
   * of the same directory somewhere else.
   */
  async function openPanel(tab: TerminalTab): Promise<void> {
    // Nothing to open when the panel is already there, is on its way, or is holding a browser that
    // nobody has closed yet — the last of those is a failed *close*, which the toggle retries.
    if (tab.panel && (tab.panel.status !== "failed" || tab.panel.handle !== null)) {
      return;
    }
    tab.panel = { handle: null, status: "opening", failure: null };

    const holder = tabs.value.find((entry) => entry.session.id === tab.session.id && browserHandleOf(entry) !== null);
    const adopted = holder ? browserHandleOf(holder) : null;
    if (holder && adopted !== null) {
      tab.panel = { handle: adopted, status: "open", failure: null };
      forget(holder.id);
      return;
    }

    try {
      acceptPanel(tab, await files.open(tab.session.id));
    } catch (error) {
      tab.panel = { handle: null, status: "failed", failure: asShellError(error, "files_open") };
    }
  }

  /**
   * Closes the panel and the browser in it.
   *
   * The browser goes through the backend first, and a refusal keeps both: a browser the backend
   * would not close is still open on the far side, so a panel that vanished would leave a
   * directory nobody can see and a session nobody can close.
   */
  async function closePanel(tab: TerminalTab): Promise<boolean> {
    const panel = tab.panel;
    if (!panel) {
      return true;
    }
    if (!panel.handle) {
      tab.panel = null;
      return true;
    }
    const failure = await files.close(panel.handle);
    if (failure) {
      tab.panel = { ...panel, status: "failed", failure };
      return false;
    }
    tab.panel = null;
    return true;
  }

  /** The toggle: what is there goes, and what is not comes. */
  async function togglePanel(tab: TerminalTab): Promise<void> {
    if (tab.panel) {
      await closePanel(tab);
      return;
    }
    await openPanel(tab);
  }

  /**
   * Takes the panel off a tab that is being dropped rather than closed.
   *
   * Not awaited and not shown, and here is why: the tab is going, so there is nothing left on
   * screen that could hold a refusal. The call is still made, because what is behind the panel is
   * a live SFTP session that has to be told to stop rather than one that will be missed.
   */
  function releasePanel(tab: TerminalTab): void {
    const handle = tab.panel?.handle;
    tab.panel = null;
    if (handle) {
      void files.close(handle);
    }
  }

  /** The handles through which a tab is drawing a file browser, whichever half of the mode it is. */
  function browserHandleOf(tab: WorkspaceTab): string | null {
    if (tab.kind === "files") {
      return tab.handle;
    }
    // A page about a host draws no browser, so there is nowhere for a dropped file to land.
    if (tab.kind === "inspect") {
      return null;
    }
    return tab.panel?.handle ?? null;
  }

  /**
   * The shell in a terminal is somewhere else now: take the file panel under it along.
   *
   * Only the panel. A file tab is somebody browsing — they may well be looking somewhere other than
   * where their shell is, on purpose — while the panel is attached to the terminal, and following
   * is what attached means. A terminal with no panel has nothing to take along, and nothing to
   * report: this is a convenience for a mode that is on screen, not a record of where shells are.
   */
  async function followDirectory(handle: string, directory: string): Promise<void> {
    const tab = tabs.value.find((entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === handle);
    const panel = tab?.panel?.handle;
    if (!panel) {
      return;
    }
    await files.followDirectory(panel, directory);
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

  /**
   * Reconnects a tab by asking for the same thing again.
   *
   * The shape comes from the settings, like the first open did: a reconnect is "that again", and
   * what the person had on screen at the moment it died — a panel they had closed, or opened — is
   * not a better answer than the one they chose in the settings.
   */
  async function reconnect(id: string): Promise<void> {
    const tab = tabs.value.find((entry) => entry.id === id);
    if (!tab) {
      return;
    }
    const { session } = tab;
    if (tab.kind === "files") {
      forget(id);
      await openFiles(session);
      return;
    }
    // A page has nothing to reconnect. It reads through whatever terminal the session has, and
    // starts reading again by itself the moment one is there — which is why it offers no button.
    if (tab.kind === "inspect") {
      return;
    }

    // The button only appears on a terminal that is not open, so there is nothing here to close
    // through the backend: the panel's browser is told to stop and the tab goes regardless.
    releasePanel(tab);
    forget(id);
    await open(session);
  }

  /**
   * Closing every tab but one, and every tab after one.
   *
   * One at a time rather than all at once: closing a tab is a conversation with the backend — a panel
   * and a channel to let go of, and a refusal to show if either will not go — and five of those
   * interleaved would leave five answers arriving at once about five different tabs.
   *
   * A tab that refuses to close stays, which is the same rule the single close follows: a browser the
   * backend would not close is still open on the far side.
   */
  async function closeOthers(id: string): Promise<void> {
    // Iterating the value the loop started with is safe: closing a tab assigns a new array rather
    // than removing from this one.
    for (const tab of tabs.value) {
      if (tab.id !== id) {
        await closeTab(tab.id);
      }
    }
  }

  async function closeToTheRight(id: string): Promise<void> {
    const index = tabs.value.findIndex((entry) => entry.id === id);
    if (index < 0) {
      return;
    }
    for (const tab of tabs.value.slice(index + 1)) {
      await closeTab(tab.id);
    }
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
    openInspect,
    openPanel,
    closePanel,
    togglePanel,
    browserHandleOf,
    followDirectory,
    confirmTrust,
    confirmReplace,
    dismiss,
    closeTab,
    closeOthers,
    closeToTheRight,
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
