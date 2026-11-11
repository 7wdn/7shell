/**
 * Hand-written mirror of the assistant's wire types.
 *
 * One rule shapes this file the way `mcp.ts` is shaped by its own: **there is no key here, and no
 * way to add one.** A provider's key lives in the vault under the provider's `id`, and the only
 * question the interface may ask about it is `hasKey` — which is `null`, not `false`, while the
 * vault is locked. "Could not tell right now" and "there is none" are two answers, and a page that
 * drew the second for the first would be telling somebody their key had been lost.
 *
 * Everything else here is a name of something the host says: the events one run emits, the stop
 * reasons a run can end with, and the shape of a request. The fields are camelCase because that is
 * what crosses the boundary (`rename_all = "camelCase"` on the Rust side).
 */

import type { AiProviderConfig, AiProviderKind } from "@/types/app";

/** Re-exported so a caller that has one import does not need two for the same subject. */
export type { AiProviderConfig, AiProviderKind };

/**
 * One provider as the settings page draws it: the configuration plus whether a key is held.
 *
 * The Rust view flattens the configuration, so this extends it rather than nesting it — a view *is*
 * the configuration with one more fact, and a nested copy would make every reader write `.config.`
 * before it could say where a request goes.
 */
export interface AiProviderView extends AiProviderConfig {
  /** `true` a key is held, `false` none is, `null` the vault is locked and nobody can say. */
  readonly hasKey: boolean | null;
}

/**
 * What a connection test answered.
 *
 * Not a rejection when the provider refused: "the key is wrong" is the answer to the question that
 * was asked, and a rejection would make the page draw it as a failure of the test rather than as its
 * result. `code` and `messageKey` are the envelope's halves, carried through so the page says the
 * same sentence every other failure says.
 */
export interface AiProviderTestResult {
  readonly ok: boolean;
  readonly code?: string;
  readonly messageKey?: string;
  readonly messageParams?: Readonly<Record<string, string>>;
}

/**
 * The terminal as it travels with a question.
 *
 * `terminalText` is the prepared snapshot itself — already stripped, truncated and redacted in the
 * window — which is what makes the preview in the chat the exact text that is sent. It is
 * deliberately a string rather than a handle: the host never reads a buffer of its own, and the
 * interface never names a path.
 */
export interface AiChatContext {
  readonly sessionId?: string;
  readonly terminalText?: string;
}

/** One question, as the window asks it. `providerId` absent means "the one that is active". */
export interface AiChatRequest {
  readonly text: string;
  readonly providerId?: string;
  readonly context?: AiChatContext;
}

/** What a send answered once the run exists: every event carries this id from then on. */
export interface AiChatStarted {
  readonly runId: string;
}

/** The usage one run reports when it ends, summed over the turns it took. */
export interface AiUsage {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cacheReadTokens?: number;
}

/** Why a run ended. `error` and `cancelled` are endings, not failures of the transport. */
export type AiStopReason = "endTurn" | "maxTurns" | "cancelled" | "error";

/** A run as the window tracks it: its id, and whether it is still producing. */
export interface AiRunState {
  readonly runId: string;
  readonly active: boolean;
}

/** One slice of the assistant's own text, addressable so it can be appended to in place. */
export interface AiDeltaEvent {
  readonly runId: string;
  readonly messageId: string;
  readonly blockIndex: number;
  readonly text: string;
}

/** A tool the assistant asked to run. `input` is whatever the model produced; never a credential. */
export interface AiToolCallEvent {
  readonly runId: string;
  readonly callId: string;
  readonly tool: string;
  readonly input: unknown;
  readonly sessionId?: string;
}

/** How one tool call ended. `preview` is the host's truncated rendering; it may be empty. */
export interface AiToolResultEvent {
  readonly runId: string;
  readonly callId: string;
  readonly tool: string;
  readonly ok: boolean;
  readonly code?: string;
  readonly preview: string;
  readonly truncated: boolean;
  readonly durationMs: number;
}

/** The end of a run: how many turns it took, and what it cost. */
export interface AiDoneEvent {
  readonly runId: string;
  readonly stopReason: AiStopReason;
  readonly turns: number;
  readonly usage?: AiUsage;
}

/** A run that failed for a reason the host could name, in the language of the error catalog. */
export interface AiErrorEvent {
  readonly runId: string;
  readonly code: string;
  readonly messageKey: string;
  readonly messageParams: Readonly<Record<string, string>>;
  readonly retryable: boolean;
}

/**
 * Everything one run says, as one union.
 *
 * The six are one shape rather than six subscriptions' worth of unrelated payloads: a reader that
 * has an event decides what to do with it by its kind, and a handler that forgot one would be a
 * compile error rather than a silent hole in the chat.
 */
export type AiChatEvent =
  | ({ readonly kind: "delta" } & AiDeltaEvent)
  | ({ readonly kind: "thinking" } & AiDeltaEvent)
  | ({ readonly kind: "toolCall" } & AiToolCallEvent)
  | ({ readonly kind: "toolResult" } & AiToolResultEvent)
  | ({ readonly kind: "done" } & AiDoneEvent)
  | ({ readonly kind: "error" } & AiErrorEvent);

/**
 * The budget a question's terminal snapshot travels under, in characters.
 *
 * About eight thousand tokens: enough for a screenful or two of output beside the question, little
 * enough that a conversation is not mostly a transcript. The host applies the same bound again —
 * this side's copy is what makes the preview truthful rather than what makes the limit hold.
 */
export const AI_CONTEXT_MAX_CHARS = 32_000;

/**
 * What one provider entry's output limit may be, in tokens.
 *
 * The model clamps further on its own side; these are the ends of the box somebody types into, so a
 * mistyped zero — which would ask for an empty answer — is corrected rather than sent.
 */
export const AI_MAX_OUTPUT_TOKENS_MIN = 1;
export const AI_MAX_OUTPUT_TOKENS_MAX = 32_768;

/**
 * The most a tool card draws. The host already truncates its own preview; this is the second bound,
 * so a preview that arrives longer than the contract allows cannot stretch the panel.
 */
export const AI_TOOL_PREVIEW_MAX_CHARS = 4_000;

/**
 * Whether a value off the wire is one of the stop reasons this build draws.
 *
 * A newer host could end a run in a way this build has no sentence for; the panel says "ended"
 * rather than drawing the raw word.
 */
export function isAiStopReason(value: unknown): value is AiStopReason {
  return value === "endTurn" || value === "maxTurns" || value === "cancelled" || value === "error";
}
