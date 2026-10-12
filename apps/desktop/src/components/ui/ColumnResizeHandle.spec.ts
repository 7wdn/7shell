/**
 * The edge between two headings: where the drag may put it, and what it says while it is there.
 *
 * A test document lays nothing out, so the geometry is the test's: the handle is told how wide the
 * column is and how far it may move, and the pointer is reported in the same made-up coordinates.
 * What is real is the arithmetic — the clamping, the pointer capture, the keys — and that is what is
 * asserted. Where the edge ends up on a screen is the e2e run's business.
 */
import { type App, createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ColumnResizeHandle from "@/components/ui/ColumnResizeHandle.vue";
import { i18n } from "@/i18n";

let host: HTMLDivElement;
let app: App | undefined;
let cell: HTMLDivElement;
const moved: number[] = [];
const reset = vi.fn();

/**
 * The width the column is drawn at, as the handle is told.
 *
 * A ref rather than a number, because that is what the handle is told by: whoever owns the column
 * hands the new width back into it and draws again — which is what makes the keys move it by one
 * step rather than by two.
 */
const width = ref(80);

/**
 * The heading the edge belongs to, which is what the handle is a separator of.
 *
 * The cell is drawn because the handle is positioned inside it — `right-0` measures from its parent
 * — and because a heading's edge belongs to a heading rather than to the strip.
 */
async function render(): Promise<void> {
  cell = document.createElement("div");
  cell.className = "relative";
  host.append(cell);

  app = createApp({
    render: () =>
      h(ColumnResizeHandle, {
        width: width.value,
        min: 56,
        label: "Resize the size column",
        onMove: (next: number) => {
          moved.push(next);
          width.value = next;
        },
        onReset: reset,
      }),
  }).use(i18n);
  app.mount(cell);
  await nextTick();
}

function handle(): HTMLElement {
  const element = host.querySelector<HTMLElement>("[data-column-divider]");
  if (!element) {
    throw new Error("the edge is not drawn");
  }
  return element;
}

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  moved.length = 0;
  reset.mockClear();
  width.value = 80;
});

afterEach(() => {
  app?.unmount();
  host.remove();
});

/** A press on the edge, with the pointer where the test says it is. */
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

describe("moving the edge between two columns", () => {
  it("reports how far the pointer moved it, measured from where the gesture started", async () => {
    await render();

    await press(100);
    await dragTo(140);
    await release();

    expect(moved.at(-1)).toBe(120);
  });

  it("will not let a column be dragged narrower than its floor", async () => {
    await render();

    await press(100);
    await dragTo(20);

    expect(moved.at(-1)).toBe(56);
  });

  it("will let it be dragged as wide as the pointer asks, with no furthest point", async () => {
    await render();

    await press(100);
    await dragTo(900);

    expect(moved.at(-1)).toBe(880);
  });

  it("reports nothing once the pointer has been let go", async () => {
    await render();

    await press(100);
    await dragTo(120);
    await release();
    const reported = moved.length;
    await dragTo(200);

    expect(moved.length).toBe(reported);
  });

  it("says where it is and how narrow it may be, and claims no furthest point", async () => {
    await render();

    expect(handle().getAttribute("aria-valuenow")).toBe("80");
    expect(handle().getAttribute("aria-valuemin")).toBe("56");
    // There is no widest: the strip scrolls, so a maximum would be a number invented somewhere.
    expect(handle().getAttribute("aria-valuemax")).toBeNull();
    expect(handle().getAttribute("role")).toBe("separator");
    expect(handle().getAttribute("aria-label")).toBe("Resize the size column");
  });

  it("moves with the arrow keys, and goes to the narrowest with Home", async () => {
    await render();

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(96);

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(80);

    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    await nextTick();
    expect(moved.at(-1)).toBe(56);

    // End is not a key this control has: there is no end to go to.
    const reported = moved.length;
    handle().dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await nextTick();
    expect(moved.length).toBe(reported);
  });

  it("hands the column back to its own width when it is double-clicked", async () => {
    await render();

    handle().dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(reset).toHaveBeenCalledTimes(1);
  });

  it("draws the boundary itself, thicker and in the accent colour while it is moved", async () => {
    await render();

    const mark = handle().querySelector("span[aria-hidden]");
    expect(mark, "the line down the boundary is what says where the edge is").not.toBeNull();
    expect(handle().querySelector(".panel-grip"), "a heading's edge carries no dots").toBeNull();
    // There before anybody points at it: a boundary nobody can see is a boundary nobody drags.
    expect(mark?.className).toContain("bg-border");
    expect(mark?.className).toContain("w-px");

    await press(100);
    await dragTo(140);

    expect(mark?.className, "and it takes the accent colour while it is dragged").not.toContain("bg-border");
    expect(mark?.className).toContain("w-0.5");
  });
});
