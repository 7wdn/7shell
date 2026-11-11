/**
 * One tool call as a card.
 *
 * The visible text is translated and the status is transient, so what an end-to-end run needs from
 * a card is the call it belongs to: `data-tool-card` plus the call id the host assigned. The rest
 * here is the card still saying what it always said.
 */
import { type App, createApp, h } from "vue";
import { afterEach, describe, expect, it } from "vitest";

import ToolCard from "@/components/assistant/ToolCard.vue";
import { i18n } from "@/i18n";
import type { AssistantToolBlock } from "@/stores/assistant";

let app: App | undefined;
let host: HTMLDivElement;

function block(overrides: Partial<AssistantToolBlock> = {}): AssistantToolBlock {
  return {
    kind: "tool",
    id: "tool:run-1:call-1",
    runId: "run-1",
    callId: "call-1",
    tool: "sshell_sessions_list",
    input: null,
    status: "running",
    preview: "",
    truncated: false,
    ...overrides,
  };
}

function mount(value: AssistantToolBlock): HTMLElement {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(ToolCard, { block: value }) }).use(i18n);
  app.mount(host);
  return host;
}

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the tool card", () => {
  it("carries the call it was drawn for", () => {
    const page = mount(block());

    const card = page.querySelector<HTMLElement>("[data-tool-card]");
    expect(card).not.toBeNull();
    expect(card?.dataset.callId).toBe("call-1");
    expect(card?.textContent).toContain("sshell_sessions_list");
  });

  it("keeps a second call apart from the first even when the tool is the same", () => {
    const page = mount(block({ id: "tool:run-1:call-2", callId: "call-2" }));

    expect(page.querySelector<HTMLElement>("[data-tool-card]")?.dataset.callId).toBe("call-2");
  });
});
