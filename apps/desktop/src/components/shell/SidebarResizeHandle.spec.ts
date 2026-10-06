/**
 * The edge beside the sidebar: where the drag may put it, and what it says while it is there.
 *
 * jsdom lays nothing out, so the geometry is the test's: the handle is told where its row starts and
 * how wide it is, and the pointer is reported in the same made-up coordinates. What is real is the
 * arithmetic — the bounds, the pointer capture, the keys — and that is what is asserted.
 */
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import SidebarResizeHandle from "@/components/shell/SidebarResizeHandle.vue";
import { i18n } from "@/i18n";

let host: HTMLDivElement;
let app: App | undefined;
let panel: HTMLDivElement;
const moved: number[] = [];
const settled = vi.fn();
const reset = vi.fn();

/** The width the panel is at, as the component is told. */
let width = 260;

/**
 * The panel, inside the row it shares with the workspace.
 *
 * The shape matters: the component reads its bounds from the row two levels up, which is the flex
 * row the sidebar and the workspace are both children of. jsdom lays nothing out, so the widths are
 * the test's — 1200 of window, and the panel's left edge at zero.
 */
async function render(): Promise<void> {
  const row = document.createElement("div");
  panel = document.createElement("div");
  row.append(panel);
  host.append(row);
  Object.defineProperty(row, "clientWidth", { value: 1200 });
  panel.getBoundingClientRect = () => ({ left: 0 }) as DOMRect;

  app = createApp({
    render: () =>
      h(SidebarResizeHandle, {
        width,
        onMove: (next: number) => {
          moved.push(next);
          width = next;
        },
        onSettle: settled,
        onReset: reset,
      }),
  }).use(i18n);
  app.mount(panel);
  await nextTick();
}

function handle(): HTMLElement {
  const element = host.querySelector<HTMLElement>("[data-sidebar-divider]");
  if (!element) {
    throw new Error("the divider is not drawn");
  }
  return element;
}

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  moved.length = 0;
  settled.mockClear();
  reset.mockClear();
  width = 260;

  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe(): void {}
      disconnect(): void {}
    },
  );
});

afterEach(() => {
  app?.unmount();
  host.remove();
  vi.unstubAllGlobals();
});

/** A press on the divider, with the pointer where the test says it is. */
async function press(clientX: number): Promise<void> {
  const element = handle();
  element.setPointerCapture = vi.fn();
  element.releasePointerCapture = vi.fn();
  element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, clientX }));
  await nextTick();
}

async function dragTo(clientX: number): Promise<void> {
  handle().dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX }));
  await nextTick();
}

async function release(): Promise<void> {
  handle().dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  await nextTick();
}

describe("moving the edge beside the sidebar", () => {
  it("puts it where the pointer is, measured from the panel's own left edge", async () => {
    await render();

    await press(260);
    await dragTo(412);
    await release();

    expect(moved.at(-1)).toBe(412);
    expect(settled).toHaveBeenCalledTimes(1);
  });

  it("will not let the list be pushed narrower than a row fits", async () => {
    await render();

    await press(260);
    await dragTo(40);

    expect(moved.at(-1)).toBe(240);
  });

  it("keeps room for the workspace, whatever the panel would like", async () => {
    // 1200 wide, 360 kept: the ceiling is 800 here, and the pointer is past it.
    await render();

    await press(260);
    await dragTo(1150);

    expect(moved.at(-1)).toBe(800);
  });

  it("says where it is, and what the floor and ceiling are", async () => {
    await render();

    expect(handle().getAttribute("aria-valuenow")).toBe("260");
    expect(handle().getAttribute("aria-valuemin")).toBe("240");
    expect(handle().getAttribute("aria-valuemax")).toBe("800");
    expect(handle().getAttribute("role")).toBe("separator");
  });

  it("moves with the arrow keys, and goes to the stops with Home and End", async () => {
    await render();

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(276);

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(240);

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(800);
    expect(settled).toHaveBeenCalledTimes(3);
  });

  it("hands back the width it started at when it is double-clicked", async () => {
    await render();

    handle().dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(reset).toHaveBeenCalledTimes(1);
  });

  it("carries a mark, so the edge can be found without hovering over it first", async () => {
    await render();

    const grip = handle().querySelector(".panel-grip");
    expect(grip, "the three dots are what says this edge moves").not.toBeNull();
  });
});
