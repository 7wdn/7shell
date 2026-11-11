/**
 * The assistant's settings page: the two things about conversations it configures.
 *
 * The provider list and the key handling are exercised through the store and the host; what is
 * asserted here is what a fresh install gets wrong if nobody looks — a retention number that reads
 * as a different number than it writes, and a protocol that is described in the tables but missing
 * from the dropdown somebody chooses it with.
 */
import { createPinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  aiProviders: vi.fn(),
  aiProviderKeySet: vi.fn(),
  aiProviderRemove: vi.fn(),
  aiProviderTest: vi.fn(),
  aiChatSend: vi.fn(),
  aiChatCancel: vi.fn(),
  aiChatReset: vi.fn(),
  aiSessionsList: vi.fn(),
  aiSessionGet: vi.fn(),
  aiSessionRename: vi.fn(),
  aiSessionDelete: vi.fn(),
  onAiDelta: vi.fn(),
  onAiThinking: vi.fn(),
  onAiToolCall: vi.fn(),
  onAiToolResult: vi.fn(),
  onAiDone: vi.fn(),
  onAiError: vi.fn(),
  onAiSessionSaved: vi.fn(),
  onAiCompacted: vi.fn(),
}));

import AiSettings from "@/components/settings/AiSettings.vue";
import { i18n } from "@/i18n";
import { aiProviders } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { AI_SESSION_RETENTION_DAYS_DEFAULT } from "@/types/app";

const providersMock = vi.mocked(aiProviders);

let app: App | undefined;
let host: HTMLDivElement;

async function mount(): Promise<void> {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(AiSettings) })
    .use(createPinia())
    .use(i18n);
  app.mount(host);
  await vi.waitFor(() => expect(providersMock).toHaveBeenCalled());
  await nextTick();
}

/** One turn of the event loop, which is what reka-ui's dismiss layer is registered on. */
async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function retention(): HTMLInputElement {
  const element = host.querySelector<HTMLInputElement>("[data-ai-retention-days]");
  if (!element) {
    throw new Error("the page has no retention field");
  }
  return element;
}

async function typeRetention(value: string): Promise<void> {
  retention().value = value;
  retention().dispatchEvent(new Event("input"));
  await nextTick();
}

/** The protocol dropdown of the open form, which is the only one carrying that id. */
function kindTrigger(): HTMLElement {
  const element = host.querySelector<HTMLElement>("#ai-provider-kind");
  if (!element) {
    throw new Error("the form has no protocol dropdown");
  }
  return element;
}

async function openKinds(): Promise<void> {
  kindTrigger().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
  kindTrigger().click();
  await settle();
}

function kindOptions(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')];
}

beforeEach(() => {
  providersMock.mockResolvedValue([]);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="listbox"]')) {
    node.remove();
  }
});

describe("the retention field", () => {
  it("starts at thirty days and keeps what is typed inside the range the host enforces", async () => {
    await mount();
    const preferences = usePreferencesStore();

    expect(retention().value).toBe(String(AI_SESSION_RETENTION_DAYS_DEFAULT));

    await typeRetention("0");
    expect(preferences.aiSessionRetentionDays).toBe(1);

    await typeRetention("400");
    expect(preferences.aiSessionRetentionDays).toBe(365);

    await typeRetention("120");
    expect(preferences.aiSessionRetentionDays).toBe(120);
    expect(preferences.dirty).toBe(true);
  });
});

describe("the protocols the form offers", () => {
  it("offers OpenAI Responses beside Chat Completions, with its own address and hint", async () => {
    await mount();

    const add = host.querySelector<HTMLButtonElement>("[data-ai-provider-add]");
    if (!add) {
      throw new Error("the page has no way to add a provider");
    }
    add.click();
    await nextTick();

    await openKinds();
    const labels = kindOptions().map((option) => option.textContent?.trim() ?? "");
    expect(labels.some((label) => label.includes("OpenAI Responses"))).toBe(true);

    const responses = kindOptions().find((option) => option.textContent?.includes("OpenAI Responses"));
    if (!responses) {
      throw new Error("the list has no OpenAI Responses entry");
    }
    responses.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "mouse" }));
    await settle();

    const baseUrl = host.querySelector<HTMLInputElement>("#ai-provider-base-url");
    expect(baseUrl?.value).toBe("https://api.openai.com/v1");
    // The hint says which endpoint that address is the base of, which is the one place a reader can
    // tell Chat Completions and Responses apart without trying both.
    expect(host.textContent).toContain("/v1/responses");
  });
});
