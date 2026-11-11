/**
 * The assistant as the window mirrors it.
 *
 * The cases worth holding still are the ones that would be quietly wrong in the panel: a reply that
 * grows in the wrong block, a run whose tail keeps arriving after it ended, a card that spins for
 * ever because its call was cancelled, and a question that was sent to a provider nobody configured.
 */
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));

/** The terminal on screen: one handle, or none for the cases about a panel with nothing to read. */
let activeHandle: string | null = "handle-1";

const TERMINAL_TEXT = "uptime\n 10:00:00 up 3 days";

vi.mock("@/stores/terminals", () => ({
  useTerminalsStore: () => ({
    activeTerminalHandle: () => activeHandle,
    activeTab: activeHandle === null ? null : { kind: "terminal", session: { id: "session-1" } },
  }),
}));

vi.mock("@/lib/terminal/buffers", () => ({
  terminalTail: vi.fn(() => TERMINAL_TEXT),
}));

vi.mock("@/lib/backend/tauri", () => ({
  aiProviders: vi.fn(),
  aiProviderKeySet: vi.fn(),
  aiProviderRemove: vi.fn(),
  aiProviderTest: vi.fn(),
  aiChatSend: vi.fn(),
  aiChatCancel: vi.fn(),
  aiChatReset: vi.fn(),
  onAiDelta: vi.fn(),
  onAiThinking: vi.fn(),
  onAiToolCall: vi.fn(),
  onAiToolResult: vi.fn(),
  onAiDone: vi.fn(),
  onAiError: vi.fn(),
}));

import { type Mock } from "vitest";

import {
  PROVIDER_KEY_STATE_KEYS,
  PROVIDER_KIND_HINT_KEYS,
  PROVIDER_KIND_LABEL_KEYS,
  SETUP_GUIDE_KEYS,
  STOP_REASON_LABEL_KEYS,
  STOP_REASON_UNKNOWN_KEY,
} from "@/lib/assistant/labels";
import {
  aiChatCancel,
  aiChatReset,
  aiChatSend,
  aiProviders,
  onAiDelta,
  onAiDone,
  onAiError,
  onAiThinking,
  onAiToolCall,
  onAiToolResult,
} from "@/lib/backend/tauri";
import { useAssistantStore } from "@/stores/assistant";
import { usePreferencesStore } from "@/stores/preferences";
import type {
  AiDeltaEvent,
  AiDoneEvent,
  AiErrorEvent,
  AiProviderView,
  AiToolCallEvent,
  AiToolResultEvent,
} from "@/types/ai";

const providersMock = vi.mocked(aiProviders);
const sendMock = vi.mocked(aiChatSend);
const cancelMock = vi.mocked(aiChatCancel);
const resetMock = vi.mocked(aiChatReset);

const LOCALES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../i18n/locales");

function provider(overrides: Partial<AiProviderView> = {}): AiProviderView {
  return {
    id: "anthropic-1",
    name: "Anthropic",
    kind: "anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-4-5",
    maxOutputTokens: 4096,
    hasKey: true,
    ...overrides,
  };
}

/** The callbacks the store registered, by channel, so a test can play the host's part. */
const handlers: {
  delta: ((payload: AiDeltaEvent) => void) | null;
  thinking: ((payload: AiDeltaEvent) => void) | null;
  toolCall: ((payload: AiToolCallEvent) => void) | null;
  toolResult: ((payload: AiToolResultEvent) => void) | null;
  done: ((payload: AiDoneEvent) => void) | null;
  error: ((payload: AiErrorEvent) => void) | null;
} = { delta: null, thinking: null, toolCall: null, toolResult: null, done: null, error: null };

/** Subscription stand-ins that hand their listener to the test instead of to the backend. */
function capture<K extends keyof typeof handlers>(mock: Mock, channel: K): void {
  mock.mockImplementation(async (listener: (typeof handlers)[K]) => {
    handlers[channel] = listener;
    return () => {};
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  activeHandle = "handle-1";
  handlers.delta = null;
  handlers.thinking = null;
  handlers.toolCall = null;
  handlers.toolResult = null;
  handlers.done = null;
  handlers.error = null;

  capture(vi.mocked(onAiDelta), "delta");
  capture(vi.mocked(onAiThinking), "thinking");
  capture(vi.mocked(onAiToolCall), "toolCall");
  capture(vi.mocked(onAiToolResult), "toolResult");
  capture(vi.mocked(onAiDone), "done");
  capture(vi.mocked(onAiError), "error");

  providersMock.mockResolvedValue([provider()]);
  sendMock.mockResolvedValue({ runId: "run-1" });
  cancelMock.mockResolvedValue(undefined);
  resetMock.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** A store whose subscriptions are already listening, which is what the frame does at startup. */
function started(): ReturnType<typeof useAssistantStore> {
  const store = useAssistantStore();
  store.initialize();
  return store;
}

/** One delta for the run on screen. */
function delta(text: string, blockIndex = 0, messageId = "msg-1", runId = "run-1"): void {
  handlers.delta?.({ runId, messageId, blockIndex, text });
}

/** The text of the entry at `index`, whichever kind it is. */
function textOf(store: ReturnType<typeof useAssistantStore>, index: number): string {
  const entry = store.entries[index];
  return entry !== undefined && (entry.kind === "text" || entry.kind === "thinking") ? entry.text : "";
}

describe("a run as it arrives", () => {
  it("appends to the block a delta belongs to rather than starting a new one", async () => {
    const store = started();
    await store.refreshProviders();
    expect(await store.send("what is this host doing?")).toBe("sent");

    delta("The host ");
    delta("has been up ");
    delta("three days.");

    const blocks = store.entries.filter((entry) => entry.kind === "text");
    expect(blocks).toHaveLength(1);
    expect(textOf(store, store.entries.length - 1)).toBe("The host has been up three days.");
  });

  it("keeps the thinking apart from the answer that follows it", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("why is it slow?");

    handlers.thinking?.({ runId: "run-1", messageId: "msg-1", blockIndex: 0, text: "checking load" });
    delta("The load average is high.");

    const kinds = store.entries.map((entry) => entry.kind).filter((kind) => kind !== "user");
    expect(kinds).toEqual(["thinking", "text"]);
  });

  it("draws a tool call as a card and closes it with the result it got", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("list the files");

    handlers.toolCall?.({ runId: "run-1", callId: "call-1", tool: "sshell_sftp_list", input: { path: "/" } });
    const running = store.entries.find((entry) => entry.kind === "tool");
    expect(running?.kind === "tool" ? running.status : null).toBe("running");

    handlers.toolResult?.({
      runId: "run-1",
      callId: "call-1",
      tool: "sshell_sftp_list",
      ok: false,
      code: "SFTP-PERMISSION-DENIED",
      preview: "permission denied",
      truncated: false,
      durationMs: 42,
    });

    const closed = store.entries.find((entry) => entry.kind === "tool");
    expect(closed?.kind === "tool" ? closed.status : null).toBe("failed");
    expect(closed?.kind === "tool" ? closed.code : null).toBe("SFTP-PERMISSION-DENIED");
    expect(closed?.kind === "tool" ? closed.durationMs : null).toBe(42);
  });

  it("closes a card whose call never reported when the run is cancelled", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("restart nginx");

    handlers.toolCall?.({ runId: "run-1", callId: "call-2", tool: "sshell_exec_run", input: {} });
    handlers.done?.({ runId: "run-1", stopReason: "cancelled", turns: 1 });

    const card = store.entries.find((entry) => entry.kind === "tool");
    expect(card?.kind === "tool" ? card.status : null).toBe("failed");
    expect(card?.kind === "tool" ? card.code : null).toBe("AI-CANCELLED");
    expect(store.running).toBe(false);
    expect(store.stopReason).toBe("cancelled");
  });

  it("stops drawing a run the moment it ended", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("hello");

    delta("first");
    handlers.done?.({ runId: "run-1", stopReason: "endTurn", turns: 1, usage: { inputTokens: 10, outputTokens: 3 } });
    delta(" — a tail that must not be drawn");

    expect(textOf(store, store.entries.length - 1)).toBe("first");
    expect(store.usage).toEqual({ inputTokens: 10, outputTokens: 3 });
    expect(store.running).toBe(false);
  });

  it("keeps a run's failure as the run's, not as the transport's", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("hello");

    handlers.error?.({
      runId: "run-1",
      code: "AI-PROVIDER-UNAVAILABLE",
      messageKey: "errors.ai.providerUnavailable",
      messageParams: {},
      retryable: true,
    });

    expect(store.runError?.code).toBe("AI-PROVIDER-UNAVAILABLE");
    expect(store.failure).toBeNull();
    expect(store.running).toBe(false);
  });

  it("takes the run's first event when it arrives before the send was answered", async () => {
    const store = started();
    await store.refreshProviders();

    let answer: (value: { runId: string }) => void = () => {};
    sendMock.mockImplementation(
      () =>
        new Promise<{ runId: string }>((resolve) => {
          answer = resolve;
        }),
    );
    const sending = store.send("hello");

    // The host starts producing as soon as it has an id, which is before the answer is awaited.
    delta("early");
    answer({ runId: "run-1" });
    await sending;

    expect(store.entries.filter((entry) => entry.kind === "text")).toHaveLength(1);
  });

  it("ignores an event for a run this chat never started", async () => {
    const store = started();
    await store.refreshProviders();

    delta("somebody else's");

    expect(store.entries).toHaveLength(0);
    expect(store.running).toBe(false);
  });

  it("does not light the panel up for a run that ended before the send was answered", async () => {
    const store = started();
    await store.refreshProviders();

    let answer: (value: { runId: string }) => void = () => {};
    sendMock.mockImplementation(
      () =>
        new Promise<{ runId: string }>((resolve) => {
          answer = resolve;
        }),
    );
    const sending = store.send("hello");

    delta("only line");
    handlers.done?.({ runId: "run-1", stopReason: "endTurn", turns: 1 });
    answer({ runId: "run-1" });
    await sending;

    expect(store.running).toBe(false);
    expect(store.stopReason).toBe("endTurn");
  });
});

describe("cancelling and clearing", () => {
  it("cancels the run the host knows, and keeps drawing until it says it ended", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("hello");

    expect(await store.cancel()).toBe(true);
    expect(cancelMock).toHaveBeenCalledWith("run-1");
    expect(store.running).toBe(true);

    handlers.done?.({ runId: "run-1", stopReason: "cancelled", turns: 1 });
    expect(store.running).toBe(false);
  });

  it("has nothing to cancel between runs", async () => {
    const store = started();
    expect(await store.cancel()).toBe(false);
    expect(cancelMock).not.toHaveBeenCalled();
  });

  it("clears the chat only once the host has forgotten it", async () => {
    const store = started();
    await store.refreshProviders();
    await store.send("hello");

    resetMock.mockRejectedValueOnce({
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      origin: { subsystem: "app" },
    });
    expect(await store.reset()).toBe(false);
    expect(store.entries.length).toBeGreaterThan(0);
    expect(store.failure?.code).toBe("APP-INTERNAL");

    resetMock.mockResolvedValueOnce(undefined);
    expect(await store.reset()).toBe(true);
    expect(store.entries).toEqual([]);
    expect(store.failure).toBeNull();
  });
});

describe("asking without a provider", () => {
  it("sends nothing when no provider is configured, and says which half is missing", async () => {
    providersMock.mockResolvedValue([]);
    const store = started();
    await store.refreshProviders();

    expect(store.setupProblem).toBe("noProvider");
    expect(await store.send("hello")).toBe("noProvider");
    expect(sendMock).not.toHaveBeenCalled();
    // Nothing was added to the chat: a question nobody could have received is not a turn.
    expect(store.entries).toHaveLength(0);
  });

  it("sends nothing when the provider has no key", async () => {
    providersMock.mockResolvedValue([provider({ hasKey: false })]);
    const store = started();
    await store.refreshProviders();

    expect(store.setupProblem).toBe("noKey");
    expect(await store.send("hello")).toBe("noKey");
    expect(sendMock).not.toHaveBeenCalled();
  });

  it("does not read a locked vault as a missing key", async () => {
    providersMock.mockResolvedValue([provider({ hasKey: null })]);
    const store = started();
    await store.refreshProviders();

    expect(store.setupProblem).toBeNull();
  });
});

describe("the terminal a question carries", () => {
  it("sends the prepared snapshot the panel previewed", async () => {
    const store = started();
    await store.refreshProviders();
    usePreferencesStore().setAiContextLines(40);

    await store.send("why is it slow?");

    expect(sendMock).toHaveBeenCalledWith({
      text: "why is it slow?",
      providerId: "anthropic-1",
      context: { sessionId: "session-1", terminalText: TERMINAL_TEXT },
    });
    const question = store.entries[0];
    expect(question?.kind === "user" ? question.context : "").toBe(TERMINAL_TEXT);
  });

  it("attaches nothing when the context is turned off", async () => {
    const store = started();
    await store.refreshProviders();
    usePreferencesStore().setAiContextEnabled(false);

    await store.send("hello");

    expect(store.context.text).toBe("");
    expect(sendMock).toHaveBeenCalledWith({ text: "hello", providerId: "anthropic-1", context: undefined });
  });

  it("attaches nothing when no terminal is on screen", async () => {
    activeHandle = null;
    const store = started();
    await store.refreshProviders();

    await store.send("hello");

    expect(store.context.text).toBe("");
    expect(sendMock).toHaveBeenCalledWith({ text: "hello", providerId: "anthropic-1", context: undefined });
  });
});

describe("the subscriptions", () => {
  it("opens all six channels and closes them when the window goes away", async () => {
    const stops = [vi.fn(), vi.fn(), vi.fn(), vi.fn(), vi.fn(), vi.fn()];
    const mocks = [
      vi.mocked(onAiDelta),
      vi.mocked(onAiThinking),
      vi.mocked(onAiToolCall),
      vi.mocked(onAiToolResult),
      vi.mocked(onAiDone),
      vi.mocked(onAiError),
    ];
    mocks.forEach((mock, index) => mock.mockResolvedValueOnce(stops[index] as never));

    const store = useAssistantStore();
    const stop = store.initialize();
    await vi.waitFor(() => expect(onAiError).toHaveBeenCalled());
    await Promise.resolve();
    stop();

    for (const [index, unsubscribe] of stops.entries()) {
      expect(unsubscribe, `channel ${index} was not closed`).toHaveBeenCalled();
    }
  });
});

/**
 * The words the panel and the settings page name through tables.
 *
 * `check-i18n.mjs` reads literal `t("…")` calls and cannot see a key that arrives through a lookup,
 * so this is the gate for those — the same one `rows.spec.ts` puts on the settings table.
 */
describe("the words the assistant draws", () => {
  const languages = ["en", "zh-CN"];

  function messages(language: string): Set<string> {
    const out = new Set<string>();
    const directory = path.join(LOCALES, language);
    for (const name of readdirSync(directory)) {
      if (!name.endsWith(".json")) {
        continue;
      }
      const flat = (value: unknown, prefix: string): void => {
        for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
          const full = prefix ? `${prefix}.${key}` : key;
          if (entry !== null && typeof entry === "object") {
            flat(entry, full);
          } else {
            out.add(full);
          }
        }
      };
      flat(JSON.parse(readFileSync(path.join(directory, name), "utf8")), "");
    }
    return out;
  }

  it("names every stop reason and the fallback, in every language", () => {
    const missing: string[] = [];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of [...Object.values(STOP_REASON_LABEL_KEYS), STOP_REASON_UNKNOWN_KEY]) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("names every protocol and its explanation, in every language", () => {
    const missing: string[] = [];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of [...Object.values(PROVIDER_KIND_LABEL_KEYS), ...Object.values(PROVIDER_KIND_HINT_KEYS)]) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("names the three key states and both halves of the setup guidance, in every language", () => {
    const missing: string[] = [];
    const keys = [
      ...Object.values(PROVIDER_KEY_STATE_KEYS),
      ...Object.values(SETUP_GUIDE_KEYS).flatMap((entry) => [entry.title, entry.body]),
      "settings.nav.ai",
      "ai.title",
    ];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of keys) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });
});
