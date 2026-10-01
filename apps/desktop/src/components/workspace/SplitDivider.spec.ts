/**
 * The drag itself, which is the part the store tests cannot see.
 *
 * A divider that answers an upward drag by moving the line down has no symptom anywhere else: the
 * store is handed a number, stores it, and is right about everything it was asked. So the arithmetic
 * between a pointer and a height is pinned here, against a region with a stated size — the one thing
 * happy-dom cannot measure for itself.
 */
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import SplitDivider from "@/components/workspace/SplitDivider.vue";
import { i18n } from "@/i18n";
import { MIN_PANE_PX, SPLIT_DIVIDER_PX } from "@/lib/workspace/split";

/** Where the two panes are, as the browser would report it: 600px tall, top edge at 0. */
const REGION_HEIGHT = 600;

/** The height the divider was told it is drawn at, which is what a drag starts from. */
const START_HEIGHT = 280;

const moves: number[] = [];
let settled = 0;
let resets = 0;

/**
 * The observer the divider registers, so a test can state the region's size and let it know.
 *
 * It throws until it is replaced: a divider that never registered an observer has an unknown
 * region, and every height below would then be computed against zero room and be wrong in a way
 * that some of the numbers would still match.
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
    const height = ref(START_HEIGHT);
    return () =>
      h("div", { class: "region" }, [
        h(SplitDivider, {
          height: height.value,
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
  app.use(i18n);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);

  const divider = separator();
  const region = divider.parentElement;
  if (!region) {
    throw new Error("the divider is not inside a region");
  }
  // No layout in this environment: the region is stated rather than measured, and the divider is
  // told about it the way a resize would tell it.
  Object.defineProperty(region, "clientHeight", { value: REGION_HEIGHT, configurable: true });
  Object.defineProperty(region, "getBoundingClientRect", {
    value: () => ({ top: 0, bottom: REGION_HEIGHT, left: 0, right: 800, width: 800, height: REGION_HEIGHT }),
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

function separator(): HTMLElement {
  const element = document.querySelector('[role="separator"]');
  if (!(element instanceof HTMLElement)) {
    throw new Error("the divider is not on the page");
  }
  return element;
}

/** A pointer at a given height on the page, as the browser would deliver it. */
function at(clientY: number): PointerEvent {
  return new PointerEvent("pointermove", { clientY, button: 0, pointerId: 1, bubbles: true });
}

/** A whole drag: grab the line, take it somewhere, let go. */
async function drag(to: number): Promise<void> {
  const divider = separator();
  divider.dispatchEvent(new PointerEvent("pointerdown", { clientY: 0, button: 0, pointerId: 1, bubbles: true }));
  divider.dispatchEvent(at(to));
  await nextTick();
}

describe("dragging the line between the terminal and the panel", () => {
  it("takes the panel's height from the pointer's distance to the floor", async () => {
    await drag(320);

    // 600px tall region, pointer 320 down it: 280px of panel under the line.
    expect(moves).toEqual([280]);
  });

  it("makes the panel taller as the pointer goes up", async () => {
    await drag(400);
    await drag(250);

    expect(moves).toEqual([200, 350]);
  });

  it("stops at the floor when the pointer is dragged past the bottom", async () => {
    await drag(900);

    expect(moves).toEqual([MIN_PANE_PX]);
  });

  it("stops where the terminal's own floor begins", async () => {
    await drag(0);

    // Both panes and the line have to add up to the region.
    expect(moves).toEqual([REGION_HEIGHT - MIN_PANE_PX - SPLIT_DIVIDER_PX]);
  });

  it("says when to write the height down, rather than on every move", async () => {
    await drag(320);
    expect(settled).toBe(0);

    separator().dispatchEvent(new PointerEvent("pointerup", { clientY: 320, button: 0, pointerId: 1, bubbles: true }));

    expect(settled).toBe(1);
  });

  it("nudges with the arrow keys, and remembers that too", async () => {
    separator().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));

    expect(moves).toEqual([START_HEIGHT + 16]);
    expect(settled).toBe(1);
  });

  it("takes the line to the stops with Home and End", async () => {
    separator().dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    separator().dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));

    expect(moves).toEqual([MIN_PANE_PX, REGION_HEIGHT - MIN_PANE_PX - SPLIT_DIVIDER_PX]);
  });

  it("puts the line back where it started on a double-click", async () => {
    separator().dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(resets).toBe(1);
  });

  it("says where the line is, so it is not only a thing to grab", async () => {
    const divider = separator();

    expect(divider.getAttribute("aria-valuenow")).toBe(String(START_HEIGHT));
    expect(divider.getAttribute("aria-valuemin")).toBe(String(MIN_PANE_PX));
    expect(divider.getAttribute("aria-valuemax")).toBe(String(REGION_HEIGHT - MIN_PANE_PX - SPLIT_DIVIDER_PX));
  });

  it("ignores a drag that started as another button", async () => {
    const divider = separator();
    divider.dispatchEvent(new PointerEvent("pointerdown", { clientY: 0, button: 2, pointerId: 1, bubbles: true }));
    divider.dispatchEvent(at(320));

    expect(moves).toEqual([]);
  });
});
