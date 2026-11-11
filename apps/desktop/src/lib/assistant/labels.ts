/**
 * The words the assistant's surfaces name through a table, as keys rather than as sentences.
 *
 * `check-i18n.mjs` reads literal `t("…")` calls and cannot see a key that arrives through a lookup,
 * so every map that turns a wire value into a sentence lives here — and `assistant.spec.ts` walks
 * these records against both locale files, the same guard the settings table has.
 */

import type { AiStopReason } from "@/types/ai";
import type { AiProviderKind } from "@/types/app";

/** How a finished run is described, keyed by the wire's own spelling. */
export const STOP_REASON_LABEL_KEYS: Record<AiStopReason, string> = {
  endTurn: "ai.stopReason.endTurn",
  maxTurns: "ai.stopReason.maxTurns",
  cancelled: "ai.stopReason.cancelled",
  error: "ai.stopReason.error",
};

/** What the provider dropdown says when the run ended in a way this build has no key for. */
export const STOP_REASON_UNKNOWN_KEY = "ai.stopReason.unknown";

/** The three protocols, as the settings page names them. */
export const PROVIDER_KIND_LABEL_KEYS: Record<AiProviderKind, string> = {
  anthropic: "ai.settings.kind.anthropic",
  openai: "ai.settings.kind.openai",
  local: "ai.settings.kind.local",
};

/** One line under the chosen protocol: what it is, and what it expects. */
export const PROVIDER_KIND_HINT_KEYS: Record<AiProviderKind, string> = {
  anthropic: "ai.settings.kind.anthropicHint",
  openai: "ai.settings.kind.openaiHint",
  local: "ai.settings.kind.localHint",
};

/** The three answers a provider row can give about its key. `null` is not one of them. */
export const PROVIDER_KEY_STATE_KEYS = {
  saved: "ai.settings.providers.keySaved",
  missing: "ai.settings.providers.keyMissing",
  locked: "ai.settings.providers.keyLocked",
} as const;

/**
 * What the panel says before a question can be asked, by the half that is missing.
 *
 * A table rather than a key assembled from the value in the template: a key built at runtime is one
 * no checker can see, which is how a missing translation becomes a raw key in the window.
 */
export const SETUP_GUIDE_KEYS: Record<"noProvider" | "noKey", { readonly title: string; readonly body: string }> = {
  noProvider: { title: "ai.setup.noProvider.title", body: "ai.setup.noProvider.body" },
  noKey: { title: "ai.setup.noKey.title", body: "ai.setup.noKey.body" },
};
