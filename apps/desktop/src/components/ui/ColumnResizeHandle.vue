<script setup lang="ts">
/**
 * The edge between one column and the next, and the drag that moves it.
 *
 * It owns no width of its own: it reports where the edge was put, and whoever owns the columns
 * decides what that means — here, that the column's own floor was not crossed. What it owns is the
 * gesture: the pointer capture, the keys, and the mark it draws while it is in use.
 *
 * **There is no ceiling.** A column may be dragged as wide as somebody likes; what happens when the
 * columns no longer fit is the strip's own scrollbar, which is how a file manager behaves and how a
 * long file name gets read. So `aria-valuemax` is not set and there is no End key: both would be
 * claiming a furthest point that does not exist. Home still goes to the floor, and the arrows nudge.
 *
 * **No three-dot grip, unlike the two panel dividers.** Those stand at the edge of a panel, where
 * nothing else says the edge can be moved, so they carry a mark that is always there. A boundary
 * between two headings is legible as one already, and what this draws instead is exactly that: a
 * hairline down the heading strip at the boundary, always visible, which is where the boundary
 * between two columns reads as one and where the edge is grabbed. It is the whole mark — there is no
 * second one — and it thickens and takes the accent colour while the pointer is on it or while it is
 * dragged, so "this line can be moved" is said before anybody touches it.
 *
 * That line is drawn in the heading strip and not through the rows. A rule down every row is six
 * vertical lines over a list whose rows already carry a rule of their own and a band under the
 * pointer, and what it would be saying — where one column ends — is said by the headings above them,
 * which is where anybody looks to drag one.
 *
 * There is no "settle" event, unlike those two. A divider reports the end of a drag because its owner
 * writes the size down when it ends; a column's width is written nowhere but this run's store, so
 * there is no moment worth marking.
 */
import { computed, ref } from "vue";

import { movableWidth } from "@/lib/columns";
import { cn } from "@/lib/utils";

const props = defineProps<{
  /** How wide the column is drawn now, in pixels. */
  width: number;
  /** The narrowest the column may be drawn. */
  min: number;
  /** What this edge is: the heading of the column to its left, in a sentence. */
  label: string;
}>();

const emit = defineEmits<{
  /** The edge moved. Fires for every pointer move of a drag. */
  move: [width: number];
  /** The edge was double-clicked: the column goes back to the width its table states. */
  reset: [];
}>();

/** One press of an arrow key: about a line of text, so the movement is legible as it happens. */
const STEP_PX = 16;

const handle = ref<HTMLSpanElement | null>(null);
const dragging = ref(false);
/** Where the gesture started: the pointer, and the width it was drawn at when it went down. */
let from: { x: number; width: number } | null = null;

function moveTo(width: number): void {
  emit("move", movableWidth(width, props.min));
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) {
    return;
  }
  // A drag along the headings is a drag, not a selection: without this the strip selects its own
  // labels as the pointer sweeps over them.
  event.preventDefault();
  handle.value?.setPointerCapture(event.pointerId);
  from = { x: event.clientX, width: props.width };
  dragging.value = true;
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value || from === null) {
    return;
  }
  // A distance rather than a position: the strip is a few pixels wide and sits inside the column, so
  // the pointer is beside the edge rather than on it, and where it started is the only thing that
  // says how far the edge has been asked to move.
  moveTo(from.width + (event.clientX - from.x));
}

function onPointerUp(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  dragging.value = false;
  from = null;
  handle.value?.releasePointerCapture(event.pointerId);
}

function onKeydown(event: KeyboardEvent): void {
  switch (event.key) {
    case "ArrowLeft":
      moveTo(props.width - STEP_PX);
      break;
    case "ArrowRight":
      moveTo(props.width + STEP_PX);
      break;
    case "Home":
      moveTo(props.min);
      break;
    default:
      return;
  }
  event.preventDefault();
}

const line = computed(() =>
  cn(
    "pointer-events-none absolute inset-y-0 right-0 transition-all",
    dragging.value ? "w-0.5 bg-primary" : "w-px bg-border group-hover:w-0.5 group-hover:bg-primary/70",
  ),
);
</script>

<template>
  <span
    ref="handle"
    data-column-divider
    class="group absolute inset-y-0 right-0 z-10 w-2 cursor-col-resize touch-none rounded-sm outline-none select-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
    role="separator"
    aria-orientation="vertical"
    tabindex="0"
    :title="props.label"
    :aria-label="props.label"
    :aria-valuemin="props.min"
    :aria-valuenow="props.width"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeydown"
    @dblclick="emit('reset')"
  >
    <!--
      The boundary itself: a hairline down the heading strip, always there, thickening and taking the
      accent colour while the pointer is over it or while the edge is dragged. A strip lies *inside*
      the column rather than across its edge, and this sits on the edge itself — across it would put
      half the strip outside the line the columns are drawn on, which the last column has no room for:
      its edge is the line's own edge, where the scroller clips the outer half away and the pointer
      lands on the scroller instead of on the strip. One shape for every column, and every one of them
      grabbable along its whole width.
    -->
    <span :class="line" aria-hidden="true" />
  </span>
</template>
