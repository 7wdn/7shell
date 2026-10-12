/**
 * The drag itself, which is the part the store tests cannot see.
 *
 * A divider that answers a drag to the right by making its pane narrower has no symptom anywhere
 * else: the store is handed a number, stores it, and is right about everything it was asked. So the
 * arithmetic between a pointer and a width is pinned here, against a row with a stated place and
 * width — the two things happy-dom cannot measure for itself — and it is measured from that row's
 * own left edge rather than as a movement, so the case below is also what says which pane is the one
 * being sized.
 */
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import SideDivider from "@/components/ui/SideDivider.vue";
import { MIN_SIDE_PANE_PX, SIDE_DIVIDER_PX, SIDE_PANE_KEEP_PX } from "@/lib/workspace/split";

/** How much room the two panes and the line have between them. */
const ROW_WIDTH = 900;

/** Where the row starts on the page: the pane's width is a distance from this, not from zero. */
const ROW_LEFT = 100;

/** The width the divider was told it is drawn at, which is what a drag starts from. */
const START_WIDTH = 224;

const moves: number[] = [];
let settled = 0;
let resets = 0;

/**
 * The observer the divider registers, so a test can state the row's width and let it know.
 *
 * It throws until it is replaced: a divider that never registered an observer has an unknown row,
 * and every width below would then be computed against zero room and be wrong in a way that some of
 * the numbers would still match.
 */
let announceResize: () => void = () => {
  throw new Error("the divider registered no observer");
};

class StubResizeObserver {
  constructor(callback: () => void) {
    announceResize = callback;
  }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

const Host = defineComponent({
  setup() {
    const width = ref(START_WIDTH);
    return () =>
      h("div", { class: "row" }, [
        h(SideDivider, {
          width: width.value,
          label: "Resize the directory tree",
          onMove: (value: number) => moves.push(value),
          onSettle: () => {
            settled += 1;
          },
          onReset: () => {
            resets += 1;
          },
        }),
      ]);
  },
});

let unmount: (() => void) | undefined;

beforeAll(() => {
  vi.stubGlobal("ResizeObserver", StubResizeObserver);
});

beforeEach(async () => {
  moves.length = 0;
  settled = 0;
  resets = 0;
  announceResize = () => {
    throw new Error("the divider registered no observer");
  };

  const app = createApp(Host);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);

  const row = divider().parentElement;
  if (!row) {
    throw new Error("the divider is not inside a row");
  }
  // No layout in this environment: the row is stated rather than measured, and the divider is told
  // about it the way a resize would tell it.
  Object.defineProperty(row, "clientWidth", { value: ROW_WIDTH, configurable: true });
  Object.defineProperty(row, "getBoundingClientRect", {
    value: () => ({ left: ROW_LEFT, right: ROW_LEFT + ROW_WIDTH, width: ROW_WIDTH, top: 0, bottom: 600, height: 600 }),
    configurable: true,
  });
  announceResize();
  await nextTick();

  unmount = () => {
    app.unmount();
    root.remove();
  };
});

afterEach(() => {
  unmount?.();
  unmount = undefined;
  document.body.innerHTML = "";
});

function divider(): HTMLElement {
  const element = document.querySelector('[role="separator"]');
  if (!(element instanceof HTMLElement)) {
    throw new Error("the divider is not on the page");
  }
  return element;
}

/** A pointer at a place on the page, as the browser would deliver it. */
function at(clientX: number): PointerEvent {
  return new PointerEvent("pointermove", { clientX, button: 0, pointerId: 1, bubbles: true });
}

/** A whole drag: grab the line, take it somewhere, let go. */
async function drag(toClientX: number): Promise<void> {
  const element = divider();
  element.dispatchEvent(new PointerEvent("pointerdown", { clientX: 0, button: 0, pointerId: 1, bubbles: true }));
  element.dispatchEvent(at(toClientX));
  await nextTick();
}

describe("dragging the line between two panes side by side", () => {
  it("takes the pane's width from the pointer's distance to the row's left edge", async () => {
    await drag(460);

    // The row starts at 100, so a pointer 460 across the page is 360 into the pane.
    expect(moves).toEqual([360]);
  });

  it("makes the pane wider as the pointer goes right", async () => {
    await drag(300);
    await drag(500);

    expect(moves).toEqual([200, 400]);
  });

  it("stops at the floor when the pointer is dragged over the pane's own content", async () => {
    await drag(120);

    expect(moves).toEqual([MIN_SIDE_PANE_PX]);
  });

  it("keeps room for the pane beside it, whatever this one would like", async () => {
    await drag(5000);

    // 900 of row, 280 kept by the listing, and the line's own 8.
    expect(moves).toEqual([ROW_WIDTH - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX]);
  });

  it("says when to write the width down, rather than on every move", async () => {
    await drag(460);
    expect(settled).toBe(0);

    divider().dispatchEvent(new PointerEvent("pointerup", { clientX: 460, button: 0, pointerId: 1, bubbles: true }));

    expect(settled).toBe(1);
  });

  it("nudges with the arrow keys, and remembers that too", async () => {
    divider().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));

    expect(moves).toEqual([START_WIDTH + 16]);
    expect(settled).toBe(1);
  });

  it("takes the line to the stops with Home and End", async () => {
    divider().dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    divider().dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));

    expect(moves).toEqual([MIN_SIDE_PANE_PX, ROW_WIDTH - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX]);
  });

  it("puts the line back where it started on a double-click", async () => {
    divider().dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(resets).toBe(1);
  });

  it("says where the line is and what it is called, so it is not only a thing to grab", async () => {
    const element = divider();

    expect(element.getAttribute("aria-valuenow")).toBe(String(START_WIDTH));
    expect(element.getAttribute("aria-valuemin")).toBe(String(MIN_SIDE_PANE_PX));
    expect(
      element.getAttribute("aria-valuemax"),
      "how far it may go is what the pane beside it leaves, not a number of its own",
    ).toBe(String(ROW_WIDTH - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX));
    expect(element.getAttribute("aria-orientation")).toBe("vertical");
    // The name comes from the panel rather than from this component: two panels draw this line, and
    // "resize the directory tree" is not what the command panel's divider is.
    expect(element.getAttribute("aria-label")).toBe("Resize the directory tree");
  });

  it("carries a mark, so the edge can be found without hovering over it first", async () => {
    const grip = divider().querySelector(".panel-grip");

    expect(grip, "the three dots are what says this edge moves").not.toBeNull();
  });

  it("ignores a drag that started as another button", async () => {
    const element = divider();
    element.dispatchEvent(new PointerEvent("pointerdown", { clientX: 0, button: 2, pointerId: 1, bubbles: true }));
    element.dispatchEvent(at(460));

    expect(moves).toEqual([]);
  });
});
