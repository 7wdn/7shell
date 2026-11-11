/**
 * The assistant, as the window keeps it.
 *
 * The conversation itself lives in the host: this store is a mirror of what one run has said so far,
 * and it is rebuilt from events rather than from a history read back — which is why there is no
 * `load` and no persistence here. Clearing the chat asks the host to forget, and the next run starts
 * from nothing.
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
  type AiDeltaEvent,
  type AiDoneEvent,
  type AiErrorEvent,
  type AiProviderView,
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
 * The codes a card is left with when its call never reported.
 *
 * A cancelled run stops between asking for a tool and hearing back, and the two reasons that can
 * happen are worth telling apart: the person stopped it, or the provider did.
 */
const CANCELLED: ErrorCode = "AI-CANCELLED";
const PROVIDER_FAILED: ErrorCode = "AI-PROVIDER-UNAVAILABLE";

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

  function setOpen(next: boolean): void {
    open.value = next;
    if (next) {
      refreshContext();
      void refreshProviders();
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

    failure.value = null;
    runError.value = null;
    usage.value = null;
    stopReason.value = null;
    turns.value = null;
    completedRunId.value = null;
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
        // Nothing is attached without text: a session id on its own would say which terminal
        // somebody is looking at without carrying anything they chose to send.
        context:
          snapshot.text === ""
            ? undefined
            : { sessionId: snapshot.sessionId ?? undefined, terminalText: snapshot.text },
      });
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
   * Clears the chat.
   *
   * The host is asked first: a conversation that stayed in the host while the window forgot it
   * would be a chat whose next question continued something nobody can see. A refusal leaves the
   * rows where they are, because they are still what the host remembers.
   */
  async function reset(): Promise<boolean> {
    if (clearing.value) {
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
    refreshContext();
    return true;
  }

  /**
   * Subscribes to the six channels one run speaks on, and reads the provider list.
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
    activeProvider,
    setupProblem,
    hasMessages,
    setOpen,
    toggle,
    refreshContext,
    refreshProviders,
    send,
    cancel,
    reset,
    initialize,
  };
});
