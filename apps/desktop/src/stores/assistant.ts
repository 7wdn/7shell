/**
 * The assistant, as the window keeps it.
 *
 * A conversation lives in the host and in the vault: this store mirrors the one the panel is
 * drawing, and it is rebuilt from events while a run is producing rather than polled. Opening a
 * stored conversation is the other direction — the host hands back its messages and this side turns
 * them into the same rows a run would have drawn, so a conversation somebody returns to looks like
 * the one they left.
 *
 * Four rules shape what is kept and what is dropped:
 *
 * 1. **One run at a time, and only its events.** Every payload names the run it belongs to, and a
 *    payload for any other run is the tail of something somebody cleared — it would append a
 *    sentence to a chat it never belonged to.
 * 2. **A key is never here.** Providers arrive as views with `hasKey`, which is `null` while the
 *    vault is locked: "cannot tell" is not "there is none", and the panel says so.
 * 3. **The context shown is the context sent.** The snapshot is read out of the terminal on screen,
 *    prepared in this window, and handed to `send` unchanged — the panel draws what travelled, not
 *    a description of it.
 * 4. **A run ends in three ways and they are kept apart.** Done with a reason, done with an error,
 *    and cancelled. A failed send is not a failed run, and a cancelled run is not a failure.
 *
 * Stored conversations add one rule of their own: **the list is the host's, the rows are ours.** A
 * session that cannot be read — a locked vault, a host without the command yet — leaves the panel
 * with an empty list and a sentence, never with an exception.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { prepareTerminalContext } from "@/lib/assistant/terminalContext";
import { backend } from "@/lib/backend/api";
import type { Unlisten } from "@/lib/backend/api";
import type { ErrorCode } from "@/lib/backend/errorCodes";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { terminalTail } from "@/lib/terminal/buffers";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import {
  AI_CONTEXT_MAX_CHARS,
  AI_TOOL_PREVIEW_MAX_CHARS,
  type AiChatContext,
  type AiCompactedEvent,
  type AiContextResource,
  type AiContextSummary,
  type AiDeltaEvent,
  type AiDoneEvent,
  type AiErrorEvent,
  type AiProviderView,
  type AiSessionDetail,
  type AiSessionSavedEvent,
  type AiSessionSummary,
  type AiStopReason,
  type AiToolCallEvent,
  type AiToolResultEvent,
  type AiUsage,
  isAiStopReason,
} from "@/types/ai";

/** A question somebody asked, with the terminal snapshot that travelled with it. */
export interface AssistantQuestion {
  readonly kind: "user";
  readonly id: string;
  readonly text: string;
  readonly at: number;
  /** Empty when nothing was attached, or when the panel had no terminal to read. */
  readonly context: string;
  /** The session the attached lines came from, when there was one. */
  readonly sessionId: string | null;
}

/** A block of the assistant's own prose. `thinking` is the model's aside, shown folded away. */
export interface AssistantTextBlock {
  readonly kind: "text" | "thinking";
  readonly id: string;
  readonly runId: string;
  readonly messageId: string;
  readonly blockIndex: number;
  text: string;
  streaming: boolean;
}

/**
 * One tool call, as a card.
 *
 * The status is three answers rather than two because "still running" is the state a person waits
 * on, and a card that only turned from one thing into another would have nowhere to say it.
 */
export interface AssistantToolBlock {
  readonly kind: "tool";
  readonly id: string;
  readonly runId: string;
  readonly callId: string;
  readonly tool: string;
  readonly input: unknown;
  readonly sessionId?: string;
  status: "running" | "ok" | "failed";
  code?: string;
  preview: string;
  truncated: boolean;
  durationMs?: number;
}

/** One row of the chat, in the order it arrived. */
export type AssistantEntry = AssistantQuestion | AssistantTextBlock | AssistantToolBlock;

/** What the panel is about to attach, and what it last attached. */
export interface AssistantContextSnapshot {
  /** The session the lines were read from, when a terminal was on screen. */
  readonly sessionId: string | null;
  /** How many lines the setting asked for. Zero means nothing is attached. */
  readonly lines: number;
  /** The prepared text — stripped, redacted, capped. Exactly what `send` carries. */
  readonly text: string;
}

/** What `send` did, so the panel can say why nothing happened. */
export type AssistantSendOutcome = "sent" | "empty" | "busy" | "noProvider" | "noKey" | "failed";

const EMPTY_CONTEXT: AssistantContextSnapshot = { sessionId: null, lines: 0, text: "" };

/**
 * How many external-server resources one question may carry.
 *
 * Four is the host's own limit (`RESOURCE_MAX_COUNT` in `src-tauri/src/ai/context.rs`) and it is
 * repeated here so the panel refuses before a send that would be quietly cut. The store is the
 * authority for the window: the picker asks it, and it says no rather than growing past four.
 */
export const ASSISTANT_ATTACHMENT_LIMIT = 4;

/**
 * The codes a card is left with when its call never reported.
 *
 * A cancelled run stops between asking for a tool and hearing back, and the two reasons that can
 * happen are worth telling apart: the person stopped it, or the provider did.
 */
const CANCELLED: ErrorCode = "AI-CANCELLED";
const PROVIDER_FAILED: ErrorCode = "AI-PROVIDER-UNAVAILABLE";

/** What the panel says after the host folded the older half of a conversation into a summary. */
export interface AiCompactionNotice {
  /** How many stored messages were replaced by the summary. */
  readonly summarized: number;
  /** How many the conversation still holds in full. */
  readonly kept: number;
  /** The model could not write the summary and the host grouped-and-cut instead. */
  readonly degraded: boolean;
  readonly at: number;
}

/**
 * One stored block, as this side reads it.
 *
 * A block a newer host writes is skipped rather than guessed at: the transcript is the host's
 * document, and drawing half of an unknown shape as text would be inventing something nobody said.
 * The names are the ones `ContentBlock` serialises with — `toolUse`/`toolResult`, camelCase.
 */
interface StoredBlock {
  readonly type: string;
  readonly text?: string;
  readonly id?: string;
  readonly name?: string;
  readonly input?: unknown;
  readonly toolUseId?: string;
  readonly content?: string;
  readonly isError?: boolean;
}

function asStoredBlock(value: unknown): StoredBlock | null {
  if (value === null || typeof value !== "object") {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (typeof record.type !== "string") {
    return null;
  }
  return {
    type: record.type,
    text: typeof record.text === "string" ? record.text : undefined,
    id: typeof record.id === "string" ? record.id : undefined,
    name: typeof record.name === "string" ? record.name : undefined,
    input: record.input,
    toolUseId: typeof record.toolUseId === "string" ? record.toolUseId : undefined,
    content: typeof record.content === "string" ? record.content : undefined,
    isError: record.isError === true,
  };
}

/**
 * Turns a stored conversation back into the rows a run would have drawn.
 *
 * Three choices are deliberate:
 *
 * * **A tool card is paired with its result.** The outcome carries what only the host knows — the
 *   catalog code, the duration, whether it was cut — and the result block carries the text the card
 *   showed. Using one without the other would restore either a card with no body or a body with no
 *   status.
 * * **A call with neither is dropped.** That is the half-turn a cancelled run leaves behind, and it
 *   was never a thing the person saw finish.
 * * **A turn made only of tool results is not a question.** It is the plumbing under a card above
 *   it; drawing it would put the user's name on text they never typed.
 */
function historyEntries(detail: AiSessionDetail): AssistantEntry[] {
  const results = new Map<string, { content: string; isError: boolean }>();
  for (const message of detail.messages) {
    for (const raw of message.blocks) {
      const block = asStoredBlock(raw);
      if (block?.type === "toolResult" && block.toolUseId !== undefined) {
        results.set(block.toolUseId, { content: block.content ?? "", isError: block.isError === true });
      }
    }
  }

  const entries: AssistantEntry[] = [];
  detail.messages.forEach((message, messageIndex) => {
    const blocks = message.blocks.map(asStoredBlock).filter((block): block is StoredBlock => block !== null);

    if (message.role === "user") {
      const text = blocks
        .filter((block) => block.type === "text" && (block.text ?? "").trim() !== "")
        .map((block) => block.text ?? "")
        .join("\n");
      if (text !== "") {
        entries.push({
          kind: "user",
          id: `history:${messageIndex}`,
          text,
          // The stored document keeps no per-message clock; nothing draws this number.
          at: 0,
          // The attached lines are not part of the transcript, so a restored question shows none.
          context: "",
          sessionId: null,
        });
      }
      return;
    }

    blocks.forEach((block, blockIndex) => {
      if (block.type === "text" && (block.text ?? "") !== "") {
        const id = `history:${messageIndex}:${blockIndex}`;
        entries.push({
          kind: "text",
          id,
          runId: "history",
          messageId: id,
          blockIndex,
          text: block.text ?? "",
          streaming: false,
        });
        return;
      }
      if (block.type === "thinking" && (block.text ?? "") !== "") {
        const id = `history:${messageIndex}:${blockIndex}`;
        entries.push({
          kind: "thinking",
          id,
          runId: "history",
          messageId: id,
          blockIndex,
          text: block.text ?? "",
          streaming: false,
        });
        return;
      }
      if (block.type !== "toolUse" || block.id === undefined || block.name === undefined) {
        return;
      }
      const outcome = detail.outcomes[block.id];
      const result = results.get(block.id);
      if (outcome === undefined && result === undefined) {
        return;
      }
      const preview = (result?.content ?? "").slice(0, AI_TOOL_PREVIEW_MAX_CHARS);
      entries.push({
        kind: "tool",
        id: `history:tool:${block.id}`,
        runId: "history",
        callId: block.id,
        tool: block.name,
        input: block.input ?? null,
        status: outcome !== undefined ? (outcome.ok ? "ok" : "failed") : result?.isError === true ? "failed" : "ok",
        code: outcome?.code,
        preview,
        truncated: outcome?.truncated === true || (result?.content.length ?? 0) > AI_TOOL_PREVIEW_MAX_CHARS,
        durationMs: outcome?.durationMs,
      });
    });
  });
  return entries;
}

/**
 * A fresh identifier for a row this side invented.
 *
 * Only for local rows: an assistant block is keyed by the identifiers the host sent, so a reply that
 * arrives twice lands on the same block rather than making a second copy of it.
 */
function localId(): string {
  const random = globalThis.crypto?.randomUUID?.();
  return random ?? `${Date.now()}-${Math.round(Math.random() * 1_000_000)}`;
}

export const useAssistantStore = defineStore("assistant", () => {
  const preferences = usePreferencesStore();
  const terminals = useTerminalsStore();

  /** Whether the panel is on screen. UI state rather than a preference: it does not outlive the window. */
  const open = ref(false);
  /** The chat, in arrival order. */
  const entries = ref<AssistantEntry[]>([]);
  /** The providers the host knows, each with whether its key is held. */
  const providers = ref<AiProviderView[]>([]);
  const loadingProviders = ref(false);
  const providersLoaded = ref(false);
  /** The provider list could not be read: a settings problem rather than a run's. */
  const providerFailure = ref<ShellError | null>(null);
  /** A run is producing, or a send is in flight. */
  const running = ref(false);
  const sending = ref(false);
  const cancelling = ref(false);
  /** A clear is in flight: the button says so rather than asking the host twice. */
  const clearing = ref(false);
  /** The run this chat is drawing, or nothing between runs. */
  const activeRunId = ref<string | null>(null);
  /**
   * The run whose ending was already seen, if it ended before the send was answered.
   *
   * A run can be over before `ai_chat_send` comes back — the host answers with an id and the run
   * starts producing — so the answer must not light the panel up again for something that has
   * already finished.
   */
  const completedRunId = ref<string | null>(null);
  /** What the last finished run cost, and how it ended. */
  const usage = ref<AiUsage | null>(null);
  const stopReason = ref<AiStopReason | null>(null);
  const turns = ref<number | null>(null);
  /** The run's own failure, in the host's words. Kept until the next send. */
  const runError = ref<AiErrorEvent | null>(null);
  /** A failure of the calls themselves: listing providers, sending, cancelling, clearing. */
  const failure = ref<ShellError | null>(null);
  /** What the panel is about to attach. */
  const context = ref<AssistantContextSnapshot>(EMPTY_CONTEXT);
  /**
   * The external-server resources the next question will carry.
   *
   * These are choices somebody made one at a time, so they are not part of the terminal snapshot:
   * that is read fresh before every send, while these stay until they are sent or removed. They
   * survive a reset and a conversation switch — a resource attached for the next question is about
   * the next question, not about the rows on screen.
   */
  const attachments = ref<AiContextResource[]>([]);

  /**
   * The conversations the host has stored, newest first, and the one on screen.
   *
   * The list is read rather than kept in step by hand: a save, a rename and a delete all answer
   * with the row the host now has, and `ai:session_saved` is the same answer pushed. `current` is a
   * filter over the list rather than a second copy, so a rename cannot leave two names on screen.
   */
  const sessions = ref<AiSessionSummary[]>([]);
  const sessionsLoaded = ref(false);
  const loadingSessions = ref(false);
  /** The list could not be read: a locked vault, or a host that does not store conversations. */
  const sessionFailure = ref<ShellError | null>(null);
  const currentSessionId = ref<string | null>(null);
  /** The conversation being opened, if one is: the row says so rather than opening twice. */
  const openingSessionId = ref<string | null>(null);
  /** What the host last said about folding the older half away, until the panel is asked again. */
  const compaction = ref<AiCompactionNotice | null>(null);
  /** The summary a stored conversation already had when it was opened. */
  const contextSummary = ref<AiContextSummary | null>(null);

  /** The provider a new question goes to: the chosen one, or the first configured. */
  const activeProvider = computed<AiProviderView | null>(() => {
    const chosen = preferences.aiDefaultProviderId;
    if (chosen !== null) {
      const match = providers.value.find((entry) => entry.id === chosen);
      if (match) {
        return match;
      }
    }
    return providers.value[0] ?? null;
  });

  /**
   * Why a question cannot be sent yet, if it cannot.
   *
   * `noKey` is only said when the host answered that there is none. A locked vault answers `null`,
   * and a panel that turned that into "you have not configured this" would be wrong about somebody's
   * settings — so that case is left to the send, which answers with the vault's own reason.
   */
  const setupProblem = computed<null | "noProvider" | "noKey">(() => {
    if (!providersLoaded.value) {
      return null;
    }
    if (providers.value.length === 0) {
      return "noProvider";
    }
    const active = activeProvider.value;
    // A local model server (Ollama, vLLM, LM Studio) is allowed to have no key; only the hosted
    // kinds need one, so `hasKey:false` there is the answer the panel must refuse to send with.
    return active !== null && active.kind !== "local" && active.hasKey === false ? "noKey" : null;
  });

  const hasMessages = computed(() => entries.value.length > 0);
  const contextLines = computed(() => (context.value.text === "" ? 0 : context.value.text.split("\n").length));

  /** The row for the conversation on screen, when the list has one. */
  const currentSession = computed<AiSessionSummary | null>(() => {
    const id = currentSessionId.value;
    return id === null ? null : (sessions.value.find((entry) => entry.id === id) ?? null);
  });

  /**
   * Reads the terminal on screen into the snapshot the panel draws.
   *
   * Called when the panel opens, when the tab on screen changes, and before every send — a buffer is
   * not a reactive value, so the only honest moment to read it is the one where somebody is looking
   * or asking.
   */
  function refreshContext(): void {
    const lines = preferences.aiContextEnabled ? preferences.aiContextLines : 0;
    const handle = terminals.activeTerminalHandle();
    const tab = terminals.activeTab;
    const sessionId = tab?.kind === "terminal" ? tab.session.id : null;
    if (lines <= 0 || handle === null) {
      context.value = { sessionId, lines: Math.max(lines, 0), text: "" };
      return;
    }
    const raw = terminalTail(handle, lines);
    context.value = {
      sessionId,
      lines,
      text: prepareTerminalContext(raw, lines, AI_CONTEXT_MAX_CHARS),
    };
  }

  /**
   * Attaches one resource to the next question.
   *
   * A resource already attached is replaced rather than added twice: it is the same URI on the same
   * server, and the newer read is the one the person just looked at. The fifth is refused, because
   * the host would cut it and a chip that vanished on send would be a lie about what travelled.
   */
  function attachResource(resource: AiContextResource): boolean {
    const index = attachments.value.findIndex(
      (entry) => entry.server === resource.server && entry.uri === resource.uri,
    );
    if (index === -1 && attachments.value.length >= ASSISTANT_ATTACHMENT_LIMIT) {
      return false;
    }
    attachments.value =
      index === -1
        ? [...attachments.value, resource]
        : attachments.value.map((entry, position) => (position === index ? resource : entry));
    return true;
  }

  /** Takes one attachment back, named the way the picker names it. */
  function removeAttachment(server: string, uri: string): void {
    attachments.value = attachments.value.filter((entry) => !(entry.server === server && entry.uri === uri));
  }

  /** Asks the host which providers exist, and what it knows about their keys. */
  async function refreshProviders(): Promise<void> {
    if (loadingProviders.value) {
      return;
    }
    loadingProviders.value = true;
    providerFailure.value = null;
    try {
      providers.value = await backend.ai.providers();
    } catch (error) {
      providerFailure.value = asShellError(error, "ai_providers");
    } finally {
      providersLoaded.value = true;
      loadingProviders.value = false;
    }
  }

  /** Puts one row into the list, in place when it is already there. */
  function upsertSummary(summary: AiSessionSummary): void {
    const index = sessions.value.findIndex((entry) => entry.id === summary.id);
    sessions.value =
      index === -1
        ? [summary, ...sessions.value]
        : sessions.value.map((entry, position) => (position === index ? summary : entry));
  }

  /**
   * Reads the stored conversations.
   *
   * A failure is the list's own rather than the chat's: a locked vault means nobody can tell which
   * conversations exist, and that must not look like a run that failed. The rows already on screen
   * are kept — a transient failure is not evidence they were deleted.
   */
  async function loadSessions(): Promise<void> {
    if (loadingSessions.value) {
      return;
    }
    loadingSessions.value = true;
    sessionFailure.value = null;
    try {
      sessions.value = await backend.ai.sessions();
      sessionsLoaded.value = true;
    } catch (error) {
      sessionFailure.value = asShellError(error, "ai_sessions_list");
    } finally {
      loadingSessions.value = false;
    }
  }

  /** Everything one conversation owns, put back to "nothing has been said yet". */
  function clearChat(): void {
    entries.value = [];
    activeRunId.value = null;
    completedRunId.value = null;
    running.value = false;
    sending.value = false;
    cancelling.value = false;
    usage.value = null;
    stopReason.value = null;
    turns.value = null;
    runError.value = null;
    compaction.value = null;
    contextSummary.value = null;
  }

  /**
   * Opens one stored conversation and draws its rows.
   *
   * `report` says whether a refusal belongs to the chat (somebody asked for this conversation) or
   * to the list (the window adopted the newest one at startup and it was not readable). The rows on
   * screen are only replaced once the host has answered, so a conversation that cannot be read
   * leaves the one that can still visible.
   */
  async function hydrateSession(sessionId: string, report: boolean): Promise<boolean> {
    if (running.value || sending.value || openingSessionId.value !== null) {
      return false;
    }
    openingSessionId.value = sessionId;
    if (report) {
      failure.value = null;
    }
    try {
      const detail = await backend.ai.session(sessionId);
      entries.value = historyEntries(detail);
      currentSessionId.value = detail.summary.id;
      upsertSummary(detail.summary);
      contextSummary.value = detail.contextSummary ?? null;
      compaction.value = null;
      activeRunId.value = null;
      completedRunId.value = null;
      running.value = false;
      usage.value = null;
      stopReason.value = null;
      turns.value = null;
      runError.value = null;
      sessionFailure.value = null;
      return true;
    } catch (error) {
      const shellError = asShellError(error, "ai_session_get");
      if (report) {
        failure.value = shellError;
      } else {
        sessionFailure.value = shellError;
      }
      return false;
    } finally {
      openingSessionId.value = null;
    }
  }

  /** Opens one stored conversation somebody picked from the list. */
  async function openSession(sessionId: string): Promise<boolean> {
    return hydrateSession(sessionId, true);
  }

  /**
   * Starts a conversation with nothing in it.
   *
   * The host is asked first and with no id, which is its way of saying "write the one you have down
   * and begin again". A refusal leaves the rows where they are, because they are still what the
   * host remembers.
   */
  async function newSession(): Promise<boolean> {
    if (running.value || sending.value || clearing.value) {
      return false;
    }
    clearing.value = true;
    failure.value = null;
    try {
      await backend.ai.reset();
    } catch (error) {
      failure.value = asShellError(error, "ai_chat_reset");
      return false;
    } finally {
      clearing.value = false;
    }
    clearChat();
    currentSessionId.value = null;
    refreshContext();
    void loadSessions();
    return true;
  }

  /** Renames a conversation. The answer replaces the row, so the list and the title cannot disagree. */
  async function renameSession(sessionId: string, name: string): Promise<boolean> {
    const trimmed = name.trim();
    if (trimmed === "") {
      return false;
    }
    failure.value = null;
    try {
      upsertSummary(await backend.ai.renameSession(sessionId, trimmed));
      return true;
    } catch (error) {
      failure.value = asShellError(error, "ai_session_rename");
      return false;
    }
  }

  /** Deletes a conversation, and with it the rows on screen when it is the one being drawn. */
  async function deleteSession(sessionId: string): Promise<boolean> {
    if (running.value || sending.value) {
      return false;
    }
    failure.value = null;
    try {
      await backend.ai.deleteSession(sessionId);
    } catch (error) {
      failure.value = asShellError(error, "ai_session_delete");
      return false;
    }
    sessions.value = sessions.value.filter((entry) => entry.id !== sessionId);
    if (currentSessionId.value === sessionId) {
      clearChat();
      currentSessionId.value = null;
      refreshContext();
    }
    return true;
  }

  /** Puts the compaction notice away. The conversation itself is unchanged. */
  function dismissCompaction(): void {
    compaction.value = null;
  }

  /**
   * Adopts the newest stored conversation at startup.
   *
   * A window that restarted should show the conversation it was in rather than an empty panel next
   * to a list of everything that was said. It happens once: a conversation somebody emptied or
   * started over is not reopened behind them.
   */
  async function adoptNewestSession(): Promise<void> {
    await loadSessions();
    if (entries.value.length > 0 || currentSessionId.value !== null) {
      return;
    }
    const newest = [...sessions.value].sort((left, right) => right.updatedAt - left.updatedAt)[0];
    if (newest !== undefined) {
      await hydrateSession(newest.id, false);
    }
  }

  function setOpen(next: boolean): void {
    open.value = next;
    if (next) {
      refreshContext();
      void refreshProviders();
      void loadSessions();
    }
  }

  function toggle(): void {
    setOpen(!open.value);
  }

  /**
   * Whether an event belongs to the run this chat is drawing.
   *
   * A run's first event can arrive before `send` has been answered — the host starts producing as
   * soon as it has an id — so the first event of a send that is still in flight adopts the run
   * rather than being dropped as somebody else's.
   */
  function accepts(runId: string): boolean {
    if (activeRunId.value === null) {
      if (!sending.value) {
        return false;
      }
      activeRunId.value = runId;
      running.value = true;
      return true;
    }
    return activeRunId.value === runId;
  }

  /** Stops the rows that say they are still producing, whichever way the run ended. */
  function settleStreaming(): void {
    for (const entry of entries.value) {
      if (entry.kind === "text" || entry.kind === "thinking") {
        entry.streaming = false;
      }
    }
  }

  /**
   * Closes the cards whose call never reported.
   *
   * A cancelled run can end between asking for a tool and hearing back, and a card left spinning
   * would be the panel claiming something is still happening.
   */
  function settleTools(code: string | undefined): void {
    for (const entry of entries.value) {
      if (entry.kind === "tool" && entry.status === "running") {
        entry.status = "failed";
        entry.code = code;
      }
    }
  }

  function finish(): void {
    running.value = false;
    activeRunId.value = null;
    cancelling.value = false;
  }

  function appendText(kind: "text" | "thinking", event: AiDeltaEvent): void {
    if (!accepts(event.runId)) {
      return;
    }
    const id = `${kind}:${event.runId}:${event.messageId}:${event.blockIndex}`;
    const existing = entries.value.find((entry) => entry.id === id);
    if (existing && (existing.kind === "text" || existing.kind === "thinking")) {
      existing.text += event.text;
      existing.streaming = true;
      return;
    }
    entries.value.push({
      kind,
      id,
      runId: event.runId,
      messageId: event.messageId,
      blockIndex: event.blockIndex,
      text: event.text,
      streaming: true,
    });
  }

  function receiveToolCall(event: AiToolCallEvent): void {
    if (!accepts(event.runId)) {
      return;
    }
    const id = `tool:${event.runId}:${event.callId}`;
    if (entries.value.some((entry) => entry.id === id)) {
      return;
    }
    entries.value.push({
      kind: "tool",
      id,
      runId: event.runId,
      callId: event.callId,
      tool: event.tool,
      input: event.input,
      sessionId: event.sessionId,
      status: "running",
      preview: "",
      truncated: false,
    });
  }

  function receiveToolResult(event: AiToolResultEvent): void {
    if (!accepts(event.runId)) {
      return;
    }
    const id = `tool:${event.runId}:${event.callId}`;
    let block = entries.value.find((entry) => entry.id === id);
    if (!block || block.kind !== "tool") {
      // A result whose call was never announced: still worth drawing, because the tool ran.
      block = {
        kind: "tool",
        id,
        runId: event.runId,
        callId: event.callId,
        tool: event.tool,
        input: null,
        status: "running",
        preview: "",
        truncated: false,
      };
      entries.value.push(block);
    }
    block.status = event.ok ? "ok" : "failed";
    block.code = event.code;
    block.preview = event.preview.slice(0, AI_TOOL_PREVIEW_MAX_CHARS);
    block.truncated = event.truncated || event.preview.length > AI_TOOL_PREVIEW_MAX_CHARS;
    block.durationMs = event.durationMs;
  }

  function receiveDone(event: AiDoneEvent): void {
    if (!accepts(event.runId)) {
      return;
    }
    settleStreaming();
    stopReason.value = isAiStopReason(event.stopReason) ? event.stopReason : null;
    turns.value = event.turns;
    usage.value = event.usage ?? null;
    settleTools(
      stopReason.value === "cancelled" ? CANCELLED : stopReason.value === "error" ? PROVIDER_FAILED : undefined,
    );
    completedRunId.value = event.runId;
    finish();
  }

  function receiveError(event: AiErrorEvent): void {
    if (!accepts(event.runId)) {
      return;
    }
    runError.value = event;
    stopReason.value = "error";
    settleStreaming();
    settleTools(event.code);
    completedRunId.value = event.runId;
    finish();
  }

  /**
   * A conversation was written down.
   *
   * The list only gains a timestamp, so a conversation that is new to this window means the list is
   * out of date and is read again. The id is adopted when the send has not answered yet — the save
   * can arrive before it, the same way a run's first delta can.
   */
  function receiveSessionSaved(event: AiSessionSavedEvent): void {
    if (sessions.value.some((entry) => entry.id === event.sessionId)) {
      sessions.value = sessions.value.map((entry) =>
        entry.id === event.sessionId ? { ...entry, updatedAt: event.updatedAt } : entry,
      );
    } else {
      void loadSessions();
    }
    if (currentSessionId.value === null && (sending.value || running.value)) {
      currentSessionId.value = event.sessionId;
    }
  }

  /**
   * The older half of a conversation became a summary.
   *
   * A compaction for a conversation this panel is not drawing is somebody else's — a run from before
   * a switch, or another window in a build that ever has one — and is dropped rather than shown over
   * a chat it does not describe.
   */
  function receiveCompacted(event: AiCompactedEvent): void {
    const ours =
      event.sessionId === currentSessionId.value ||
      (currentSessionId.value === null && (sending.value || running.value));
    if (!ours) {
      return;
    }
    currentSessionId.value = event.sessionId;
    compaction.value = {
      summarized: event.summarizedMessages,
      kept: event.keptMessages,
      degraded: event.degraded,
      at: Date.now(),
    };
  }

  /**
   * Asks a question.
   *
   * Nothing leaves the machine when there is no provider or no key: that answer is the panel's
   * guidance rather than a request that fails, which is why it is returned rather than thrown.
   */
  async function send(text: string): Promise<AssistantSendOutcome> {
    const trimmed = text.trim();
    if (trimmed === "" || sending.value) {
      return trimmed === "" ? "empty" : "busy";
    }
    if (running.value) {
      return "busy";
    }
    const blocked = setupProblem.value;
    if (blocked !== null) {
      return blocked;
    }

    // Read here rather than trusting what is on screen: the preview may be a moment old, and what
    // is sent is what this call just read.
    refreshContext();
    const snapshot = context.value;
    // A resource attached for this question goes with it even when the terminal contributes nothing:
    // the two halves of the context are chosen separately, and the question may be about the
    // resource alone.
    const resources = attachments.value.map((entry) => ({ ...entry }));
    const contextPayload: AiChatContext | undefined =
      snapshot.text === "" && resources.length === 0
        ? undefined
        : {
            // The session id names the terminal the lines came from, so it travels only when lines do.
            ...(snapshot.text !== "" && snapshot.sessionId !== null ? { sessionId: snapshot.sessionId } : {}),
            ...(snapshot.text !== "" ? { terminalText: snapshot.text } : {}),
            ...(resources.length > 0 ? { resources } : {}),
          };

    failure.value = null;
    runError.value = null;
    usage.value = null;
    stopReason.value = null;
    turns.value = null;
    completedRunId.value = null;
    // The last notice was about the conversation as it stood; a new question starts a new account
    // of it, and a notice left above the answer would describe a compaction this turn did not do.
    compaction.value = null;
    sending.value = true;
    entries.value.push({
      kind: "user",
      id: localId(),
      text: trimmed,
      at: Date.now(),
      context: snapshot.text,
      sessionId: snapshot.sessionId,
    });

    try {
      const started = await backend.ai.send({
        text: trimmed,
        providerId: activeProvider.value?.id,
        // The conversation this store is drawing, when there is one. Absent asks the host for a new
        // one, which is what the first question of a conversation does.
        sessionId: currentSessionId.value ?? undefined,
        // Nothing is attached without text: a session id on its own would say which terminal
        // somebody is looking at without carrying anything they chose to send.
        context: contextPayload,
      });
      // The answer names the conversation the run belongs to, which is how a new one becomes the
      // one this panel is drawing — resets and the list both need that id.
      if (started.sessionId !== undefined) {
        currentSessionId.value = started.sessionId;
      }
      // Only a send the host accepted takes the attachments with it. A refusal leaves them on
      // screen: what did not travel is still the next question's material, not a reason to attach
      // the same resource twice.
      attachments.value = [];
      // A run that finished before this answer arrived was already drawn to its end; lighting the
      // panel up again would leave it saying "replying" over nothing.
      if (completedRunId.value !== started.runId) {
        activeRunId.value = started.runId;
        running.value = true;
      }
      return "sent";
    } catch (error) {
      failure.value = asShellError(error, "ai_chat_send");
      return "failed";
    } finally {
      sending.value = false;
    }
  }

  /** Asks the run on screen to stop. The answer is whether the request reached the host. */
  async function cancel(): Promise<boolean> {
    const runId = activeRunId.value;
    if (runId === null || cancelling.value) {
      return false;
    }
    cancelling.value = true;
    failure.value = null;
    try {
      await backend.ai.cancel(runId);
      return true;
    } catch (error) {
      failure.value = asShellError(error, "ai_chat_cancel");
      return false;
    } finally {
      cancelling.value = false;
    }
  }

  /**
   * Clears the conversation on screen.
   *
   * The host is asked first: a conversation that stayed in the host while the window forgot it
   * would be a chat whose next question continued something nobody can see. A refusal leaves the
   * rows where they are, because they are still what the host remembers.
   *
   * The conversation being drawn is named, so clearing empties that one — it stays in the list, and
   * the next question continues it. Naming nothing (a panel that has not sent yet) lets the host
   * clear whatever it holds.
   */
  async function reset(): Promise<boolean> {
    if (clearing.value) {
      return false;
    }
    clearing.value = true;
    failure.value = null;
    try {
      await backend.ai.reset(currentSessionId.value ?? undefined);
    } catch (error) {
      failure.value = asShellError(error, "ai_chat_reset");
      return false;
    } finally {
      clearing.value = false;
    }
    clearChat();
    refreshContext();
    void loadSessions();
    return true;
  }

  /**
   * Subscribes to the channels a run and its conversation speak on, and reads the lists.
   *
   * Registered by the frame rather than by the panel: a run outlives a look at another page, and
   * its rows have to keep arriving while the panel is put away. The returned function drops every
   * subscription and is safe to call after the window is gone.
   */
  function initialize(): () => void {
    let stopped = false;
    const stops: Unlisten[] = [];

    void (async () => {
      try {
        const subscriptions = await Promise.all([
          backend.events.onAiDelta((event) => appendText("text", event)),
          backend.events.onAiThinking((event) => appendText("thinking", event)),
          backend.events.onAiToolCall(receiveToolCall),
          backend.events.onAiToolResult(receiveToolResult),
          backend.events.onAiDone(receiveDone),
          backend.events.onAiError(receiveError),
          backend.events.onAiSessionSaved(receiveSessionSaved),
          backend.events.onAiCompacted(receiveCompacted),
        ]);
        if (stopped) {
          for (const stop of subscriptions) {
            stop();
          }
          return;
        }
        stops.push(...subscriptions);
      } catch (error) {
        failure.value = asShellError(error, "ai_listen");
      }
    })();

    void refreshProviders();
    // A window that restarted shows the conversation it was in rather than an empty panel beside a
    // list of everything that was ever said. Failures here stay with the list: nothing about the
    // chat was asked for.
    void adoptNewestSession();

    return () => {
      stopped = true;
      for (const stop of stops) {
        stop();
      }
      stops.length = 0;
    };
  }

  return {
    open,
    entries,
    providers,
    providersLoaded,
    loadingProviders,
    providerFailure,
    running,
    sending,
    cancelling,
    clearing,
    activeRunId,
    usage,
    stopReason,
    turns,
    runError,
    failure,
    context,
    contextLines,
    attachments,
    attachResource,
    removeAttachment,
    activeProvider,
    setupProblem,
    hasMessages,
    sessions,
    sessionsLoaded,
    loadingSessions,
    sessionFailure,
    currentSessionId,
    currentSession,
    openingSessionId,
    compaction,
    contextSummary,
    setOpen,
    toggle,
    refreshContext,
    refreshProviders,
    loadSessions,
    openSession,
    newSession,
    renameSession,
    deleteSession,
    dismissCompaction,
    send,
    cancel,
    reset,
    initialize,
  };
});
