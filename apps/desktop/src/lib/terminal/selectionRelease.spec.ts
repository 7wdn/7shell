import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { guardSelectionRelease } from "./selectionRelease";

describe("a terminal's abandoned selection drag", () => {
  let surface: HTMLDivElement;
  let terminal: { modes: { mouseTrackingMode: string }; options: { macOptionClickForcesSelection?: boolean } };
  let released: ReturnType<typeof vi.fn<() => void>>;
  let cancelled: ReturnType<typeof vi.fn<() => void>>;
  let guard: ReturnType<typeof guardSelectionRelease>;
  let recovery: MouseEvent[];

  const record = (event: MouseEvent): void => {
    if (event.button === 3) {
      recovery.push(event);
    }
  };
  const down = (extra: MouseEventInit = {}): void => {
    surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, buttons: 1, ...extra }));
  };
  const move = (buttons: number): void => {
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, buttons }));
  };

  beforeEach(() => {
    vi.useFakeTimers();
    surface = document.createElement("div");
    document.body.append(surface);
    terminal = { modes: { mouseTrackingMode: "none" }, options: {} };
    released = vi.fn<() => void>();
    cancelled = vi.fn<() => void>();
    recovery = [];
    document.addEventListener("mouseup", record);
    guard = guardSelectionRelease(surface, terminal, { isMac: false, released, cancelled });
  });

  afterEach(() => {
    guard.dispose();
    document.removeEventListener("mouseup", record);
    surface.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("ends a missed release before xterm handles a button-free move", () => {
    const order: string[] = [];
    document.addEventListener("mouseup", () => order.push("released"), { once: true });
    document.addEventListener("mousemove", () => order.push("moved"), { once: true });
    down();
    move(0);
    expect(order).toEqual(["released", "moved"]);
    expect(recovery).toHaveLength(1);
    expect(recovery[0]?.buttons).toBe(0);
    expect(released).toHaveBeenCalledOnce();
    move(0);
    expect(recovery).toHaveLength(1);
  });

  it("keeps real held-button drags and their ordinary release untouched", () => {
    down();
    move(1);
    expect(recovery).toHaveLength(0);
    window.dispatchEvent(new PointerEvent("pointerup", { button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, altKey: true }));
    vi.runAllTimers();
    move(0);
    expect(recovery).toHaveLength(0);
    expect(released).toHaveBeenCalledOnce();
  });

  it("uses pointer release when its compatibility mouseup never arrives", () => {
    down();
    window.dispatchEvent(new PointerEvent("pointerup", { button: 0 }));
    expect(recovery).toHaveLength(0);
    vi.runAllTimers();
    expect(recovery).toHaveLength(1);
    expect(released).toHaveBeenCalledOnce();
  });

  it("ignores moves and presses belonging to other controls", () => {
    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, buttons: 1 }));
    move(0);
    surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 2, buttons: 2 }));
    move(0);
    expect(recovery).toHaveLength(0);
    expect(released).not.toHaveBeenCalled();
  });

  it("leaves application mouse tracking alone unless local selection is forced", () => {
    terminal.modes.mouseTrackingMode = "drag";
    down();
    move(0);
    expect(recovery).toHaveLength(0);
    down({ shiftKey: true });
    move(0);
    expect(recovery).toHaveLength(1);
  });

  it("uses xterm's Mac option modifier for forced local selection", () => {
    guard.dispose();
    guard = guardSelectionRelease(surface, terminal, { isMac: true, released, cancelled });
    terminal.modes.mouseTrackingMode = "drag";
    down({ shiftKey: true, altKey: true });
    move(0);
    expect(recovery).toHaveLength(0);
    terminal.options.macOptionClickForcesSelection = true;
    down({ altKey: true });
    move(0);
    expect(recovery).toHaveLength(1);
  });

  it.each(["blur", "pointercancel"])("ends a drag on %s without opening its toolbar", (type) => {
    down();
    window.dispatchEvent(new Event(type));
    expect(recovery).toHaveLength(1);
    expect(cancelled).toHaveBeenCalledOnce();
    expect(released).not.toHaveBeenCalled();
  });

  it("ends a drag when the page is hidden", () => {
    down();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(recovery).toHaveLength(1);
    expect(cancelled).toHaveBeenCalledOnce();
  });

  it("ends the previous drag before a new press and supports explicit tab cancellation", () => {
    down();
    down();
    expect(recovery).toHaveLength(1);
    guard.cancel();
    expect(recovery).toHaveLength(2);
    expect(cancelled).toHaveBeenCalledTimes(2);
  });

  it("cancels pending recovery and detaches listeners when disposed", () => {
    down();
    window.dispatchEvent(new PointerEvent("pointerup", { button: 0 }));
    guard.dispose();
    vi.runAllTimers();
    expect(recovery).toHaveLength(1);
    down();
    move(0);
    expect(recovery).toHaveLength(1);
    expect(released).not.toHaveBeenCalled();
  });
});
