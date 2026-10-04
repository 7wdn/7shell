/**
 * Moving a row with the mouse: press, travel, release.
 *
 * Three decisions shape this:
 *
 * 1. **It is the mouse, not the platform's drag and drop.** What is being moved is one id that the
 *    store already knows; a data transfer would carry a copy of it across a boundary that does not
 *    exist, and the platform's ghost is a picture of the row rather than the row's name.
 * 2. **Nothing happens until the pointer has actually travelled.** A press is how a row is opened
 *    and how a menu is asked for, so a drag only begins after a few pixels: below that, every event
 *    is left alone and the click underneath still works.
 * 3. **The indicator is drawn from the same answer the drop will get.** [`accepts`] is asked about
 *    every candidate before it is drawn, so the line on screen is a promise that the release keeps —
 *    a drop that would be refused draws nothing at all.
 *
 * What this owns is the pointer and the drawing hints. What a drop *means* is not here: the caller
 * turns a row into a meaning (`dropFor`), says whether it can be done, and does it.
 */
import { onBeforeUnmount, ref, type Ref } from "vue";

import { type AimedDrop, type Grabbed, type RowBox } from "@/lib/sessions/drag";
import type { DragTarget } from "@/lib/sessions/tree";

/** How far the pointer travels before a press becomes a drag, in pixels. */
const THRESHOLD = 4;
/** The band at each end of the scroller where a drag scrolls it. */
const EDGE = 36;
/** The fastest the drag scrolls, in pixels per frame, at the very edge. */
const EDGE_SPEED = 18;
/** How far outside the scroller, sideways, the pointer still counts as being in it. */
const SIDEWAYS = 32;

/** Where the drop would land, drawn as a line beside a row or a ring around one. */
export interface DropMark {
  /** The row's key, as the tree gave it. */
  readonly key: string;
  readonly style: "line-before" | "line-after" | "ring";
}

export interface TreeDragOptions {
  /** The scroller the rows live in: what is hit-tested against, and what scrolls. */
  readonly container: Ref<HTMLElement | null>;
  /** Reads the row an element belongs to, or nothing when it is not a row. */
  readonly targetOf: (element: HTMLElement) => DragTarget | null;
  /** What dropping the grabbed thing on that row would mean. */
  readonly aim: (grabbed: Grabbed, target: DragTarget, box: RowBox, pointerY: number) => AimedDrop;
  /** Whether the drop would do anything at all. What would not is not a target. */
  readonly accepts: (grabbed: Grabbed, aimed: AimedDrop) => boolean;
  readonly onDrop: (grabbed: Grabbed, aimed: AimedDrop) => void;
}

/** The row a drop decorates: a line beside the group it goes next to, or a ring around where it goes. */
export function dropMark(aimed: AimedDrop): DropMark {
  if (aimed.drop.kind === "group") {
    if (aimed.zone === "before") {
      return { key: `group:${aimed.drop.id}`, style: "line-before" };
    }
    if (aimed.zone === "after") {
      return { key: `group:${aimed.drop.id}`, style: "line-after" };
    }
    return { key: `group:${aimed.drop.id}`, style: "ring" };
  }
  // The ungrouped bucket is where both "no group" and "the root of the tree" are drawn.
  return { key: "ungrouped", style: "ring" };
}

export function useTreeDrag(options: TreeDragOptions) {
  const active = ref(false);
  const ghost = ref<{ readonly label: string; readonly x: number; readonly y: number } | null>(null);
  const mark = ref<DropMark | null>(null);

  let grabbed: Grabbed | null = null;
  let label = "";
  let from = { x: 0, y: 0 };
  let at = { x: 0, y: 0 };
  let candidate: AimedDrop | null = null;
  let frame = 0;

  function press(event: MouseEvent, item: Grabbed, name: string): void {
    if (event.button !== 0 || active.value) {
      return;
    }
    grabbed = item;
    label = name;
    from = { x: event.clientX, y: event.clientY };
    at = { ...from };
    window.addEventListener("mousemove", onMove, true);
    window.addEventListener("mouseup", onUp, true);
    window.addEventListener("keydown", onKey, true);
  }

  function onMove(event: MouseEvent): void {
    if (!grabbed) {
      return;
    }
    at = { x: event.clientX, y: event.clientY };
    if (!active.value) {
      if (Math.abs(at.x - from.x) + Math.abs(at.y - from.y) < THRESHOLD) {
        return;
      }
      begin();
    }
    // Cancelled while it is a drag: otherwise the browser starts selecting the text of every row
    // the pointer crosses, and the window fills up with a blue smear.
    event.preventDefault();
    ghost.value = { label, x: at.x, y: at.y };
    look();
  }

  function begin(): void {
    active.value = true;
    candidate = null;
    document.body.style.userSelect = "none";
    document.body.style.cursor = "grabbing";
    schedule();
  }

  /** Reads what is under the pointer and turns it into the one drop that would be accepted. */
  function look(): void {
    const container = options.container.value;
    const item = grabbed;
    if (!container || !item) {
      candidate = null;
      mark.value = null;
      return;
    }

    const under = document.elementFromPoint(at.x, at.y);
    const row = under instanceof Element ? under.closest<HTMLElement>("[data-row]") : null;
    const target = row && container.contains(row) ? options.targetOf(row) : null;
    if (!row || !target) {
      candidate = null;
      mark.value = null;
      return;
    }

    const rect = row.getBoundingClientRect();
    const box: RowBox = { top: rect.top, height: rect.height };
    const aimed = options.aim(item, target, box, at.y);
    if (!options.accepts(item, aimed)) {
      candidate = null;
      mark.value = null;
      return;
    }
    candidate = aimed;
    mark.value = dropMark(aimed);
  }

  /** Scrolls while the pointer is held near an end, and looks again after every step. */
  function schedule(): void {
    if (frame !== 0) {
      return;
    }
    frame = window.requestAnimationFrame(() => {
      frame = 0;
      if (!active.value) {
        return;
      }
      const container = options.container.value;
      if (container) {
        const rect = container.getBoundingClientRect();
        const fromTop = at.y - rect.top;
        const fromBottom = rect.bottom - at.y;
        let step = 0;
        if (fromTop < EDGE && fromTop > -EDGE) {
          step = -Math.ceil(((EDGE - fromTop) / EDGE) * EDGE_SPEED);
        } else if (fromBottom < EDGE && fromBottom > -EDGE) {
          step = Math.ceil(((EDGE - fromBottom) / EDGE) * EDGE_SPEED);
        }
        const sideways = at.x >= rect.left - SIDEWAYS && at.x <= rect.right + SIDEWAYS;
        if (step !== 0 && sideways) {
          // A step that the scroller refuses (it is already at the end) still has to look again:
          // the pointer may have been left over a row that scrolled away from under it.
          container.scrollTop += step;
          look();
        }
      }
      schedule();
    });
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      finish();
    }
  }

  function onUp(): void {
    const item = grabbed;
    const aimed = candidate;
    const dragging = active.value;
    finish();
    if (dragging && item && aimed) {
      // The press that started this is still a press: without this, letting go of a row that
      // travelled and came back would open a terminal nobody asked for.
      swallowNextClick();
      options.onDrop(item, aimed);
    }
  }

  /**
   * Stops the click the browser is about to fire for the press that started this drag.
   *
   * The listener is not put on a timer: whether the click arrives in the same task as the release or
   * in the next one is the browser's business, and a listener removed after a tick would lose that
   * race half the time. It stays until it has eaten one click, and a press anywhere else takes it
   * away again — so it cannot survive into the next thing somebody does.
   */
  function swallowNextClick(): void {
    const stop = (event: MouseEvent): void => {
      event.stopPropagation();
      event.preventDefault();
      release();
    };
    const release = (): void => {
      window.removeEventListener("click", stop, true);
      window.removeEventListener("mousedown", release, true);
    };
    window.addEventListener("click", stop, true);
    window.addEventListener("mousedown", release, true);
  }

  function finish(): void {
    window.removeEventListener("mousemove", onMove, true);
    window.removeEventListener("mouseup", onUp, true);
    window.removeEventListener("keydown", onKey, true);
    if (frame !== 0) {
      window.cancelAnimationFrame(frame);
      frame = 0;
    }
    grabbed = null;
    candidate = null;
    active.value = false;
    ghost.value = null;
    mark.value = null;
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
  }

  onBeforeUnmount(finish);

  return { active, ghost, mark, press };
}
