import { type App, createApp, h } from "vue";
import { afterEach, describe, expect, it } from "vitest";

import TerminalSplitHandle from "@/components/workspace/TerminalSplitHandle.vue";
import { i18n } from "@/i18n";
import { type TerminalDivider, splitTerminalPane, terminalGeometry } from "@/lib/workspace/terminal-layout";

let app: App | undefined;
let host: HTMLElement | undefined;
const moves: number[] = [];

function render(divider: TerminalDivider): HTMLElement {
  moves.length = 0;
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({
    render: () => h(TerminalSplitHandle, { divider, onMove: (ratio: number) => moves.push(ratio) }),
  }).use(i18n);
  app.mount(host);
  const handle = host.querySelector<HTMLElement>("[data-terminal-divider]")!;
  handle.parentElement!.getBoundingClientRect = () => ({ left: 100, top: 60 }) as DOMRect;
  return handle;
}

afterEach(() => {
  app?.unmount();
  host?.remove();
});

describe("terminal split handles", () => {
  it("measures a nested horizontal drag from its own region rather than the whole workspace", () => {
    const pair = splitTerminalPane({ kind: "pane", paneId: "a" }, "a", "b", "right", "ab");
    const triple = splitTerminalPane(pair, "b", "c", "down", "bc");
    const nested = splitTerminalPane(triple, "c", "d", "right", "cd");
    const divider = terminalGeometry(nested, 1800, 1200).dividers.find((entry) => entry.id === "cd")!;
    const handle = render(divider);
    handle.dispatchEvent(new PointerEvent("pointerdown", { button: 0, pointerId: 1 }));
    handle.dispatchEvent(new PointerEvent("pointermove", { clientX: 100 + divider.region.left + 300, pointerId: 1 }));
    handle.dispatchEvent(new PointerEvent("pointerup", { pointerId: 1 }));
    expect(moves).toHaveLength(1);
    expect(moves[0]).toBeCloseTo(300 / 888);
  });

  it("clamps a vertical drag to the space both terminals need", () => {
    const pair = splitTerminalPane({ kind: "pane", paneId: "a" }, "a", "b", "down", "ab");
    const divider = terminalGeometry(pair, 800, 600).dividers[0]!;
    const handle = render(divider);
    handle.dispatchEvent(new PointerEvent("pointerdown", { button: 0, pointerId: 1 }));
    handle.dispatchEvent(new PointerEvent("pointermove", { clientY: 2000, pointerId: 1 }));
    expect(moves).toEqual([432 / 592]);
    expect(handle.getAttribute("aria-orientation")).toBe("horizontal");
  });

  it("supports keyboard adjustments and restores equal panes on double click", () => {
    const pair = splitTerminalPane({ kind: "pane", paneId: "a" }, "a", "b", "right", "ab");
    const divider = terminalGeometry(pair, 1000, 600).dividers[0]!;
    const handle = render(divider);
    handle.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", cancelable: true }));
    handle.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", cancelable: true }));
    handle.dispatchEvent(new MouseEvent("dblclick"));
    expect(moves).toEqual([0.5 + 16 / 992, 240 / 992, 0.5]);
    expect(handle.getAttribute("aria-valuenow")).toBe("50");
  });
});
