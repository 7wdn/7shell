<script setup lang="ts">
/**
 * The line between two panes side by side, and the drag that moves it.
 *
 * It owns no width of its own: it reports where the line was put, and the panel — which is what the
 * width is about — decides what that means and writes it down. What it owns are the bounds, and the
 * fact that the drag has to be bounded by the room the two panes are inside: it watches the element
 * it sits in rather than being told a number that the window could invalidate a moment later. The
 * same bounds are applied to what is drawn, and the rule is in `lib/workspace/split.ts`.
 *
 * It reports the width of the pane on its *left*, measured from the left edge of that row rather
 * than as a delta: the row is the room there is, and a pane beside a divider that changed size
 * underneath it — a window, a tab, another divider — would drag to a width nobody asked for if the
 * gesture were counted in pointer movement instead.
 *
 * Three dots in the middle of the edge say the edge can be moved. Without them the divider is a line
 * that only shows itself to somebody whose pointer happens to cross it, which is a control nobody
 * finds by looking — and this one splits a panel somebody is already looking at, so it has to say
 * that there is a second half to it.
 *
 * Both a pointer and a keyboard can move it. A separator that only answers to a mouse is a control
 * half the people cannot use, and this one is a control like any other: arrows nudge it, Home and
 * End take it to the stops, and it says where it is.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { MIN_SIDE_PANE_PX, SIDE_DIVIDER_PX, SIDE_PANE_KEEP_PX, clampPaneWidth } from "@/lib/workspace/split";
import { cn } from "@/lib/utils";

const props = defineProps<{
  /** Where the line is now: the width of the pane on its left, in pixels. */
  width: number;
  /** What the line is called for somebody who cannot see it. */
  label: string;
}>();

const emit = defineEmits<{
  /** The line moved. Fires for every pointer move of a drag. */
  move: [width: number];
  /** The line was let go, or nudged with a key: where it ended up is worth writing down. */
  settle: [];
  /** The line was double-clicked: back to where it started. */
  reset: [];
}>();

const divider = ref<HTMLDivElement | null>(null);
const dragging = ref(false);

/**
 * How much room the two panes and this line have between them.
 *
 * Watched rather than measured once, because the number is read during a render as well as during a
 * drag: the window can be resized between the two, and `aria-valuemax` should not outlive the shape
 * it described. The divider is a direct child of the row it splits — the element it is inside is
 * exactly the two panes plus itself.
 */
const available = ref(0);
let observer: ResizeObserver | undefined;

/** One press of an arrow key: about a line of text, so the movement is legible as it happens. */
const STEP_PX = 16;

/** Where the line may be put, given the room there is now. */
function bounded(width: number): number {
  return clampPaneWidth(width, available.value, MIN_SIDE_PANE_PX);
}

/** The furthest right the line may be dragged to; what the room costs is the pane beside it. */
const ceiling = computed(() => Math.max(MIN_SIDE_PANE_PX, available.value - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX));

function moveTo(width: number): void {
  emit("move", bounded(width));
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) {
    return;
  }
  // A drag across a panel is a drag, not a selection: without this the pane on the left selects its
  // own text as the pointer sweeps over it.
  event.preventDefault();
  divider.value?.setPointerCapture(event.pointerId);
  dragging.value = true;
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  const row = divider.value?.parentElement;
  if (!row) {
    return;
  }
  // Measured from the left of the row rather than tracked as a delta: what is being sized is the
  // pane to the left, so its width is its distance from that edge.
  moveTo(event.clientX - row.getBoundingClientRect().left);
}

function onPointerUp(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  dragging.value = false;
  divider.value?.releasePointerCapture(event.pointerId);
  emit("settle");
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
      moveTo(MIN_SIDE_PANE_PX);
      break;
    case "End":
      moveTo(Number.POSITIVE_INFINITY);
      break;
    default:
      return;
  }
  event.preventDefault();
  emit("settle");
}

onMounted(() => {
  const row = divider.value?.parentElement;
  if (!row) {
    return;
  }
  available.value = row.clientWidth;
  observer = new ResizeObserver(() => {
    available.value = row.clientWidth;
  });
  observer.observe(row);
});

onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div
    ref="divider"
    data-pane-divider
    :class="
      cn(
        'group relative w-2 shrink-0 cursor-col-resize touch-none select-none outline-none',
        'focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
      )
    "
    role="separator"
    aria-orientation="vertical"
    tabindex="0"
    :aria-label="props.label"
    :aria-valuemin="MIN_SIDE_PANE_PX"
    :aria-valuemax="Math.round(ceiling)"
    :aria-valuenow="Math.round(props.width)"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeydown"
    @dblclick="emit('reset')"
  >
    <!--
      The line is drawn at the edge's own place — down the middle of this strip — and is always there
      to be grabbed: it is a target rather than a decoration, so what changes is how visible it is,
      not whether it exists.
    -->
    <span
      :class="
        cn(
          'pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 bg-border transition-all',
          dragging ? 'w-0.5 bg-primary' : 'w-px group-hover:w-0.5 group-hover:bg-primary/60',
        )
      "
    />
    <!--
      Three dots in the middle, lying along the line: the mark that says this edge can be moved.
      Without it the divider is a line that only shows itself to somebody whose pointer happens to
      cross it, which is a control nobody finds by looking.
    -->
    <span
      class="panel-grip"
      :class="dragging ? 'text-primary' : 'text-muted-foreground/50 group-hover:text-primary/70'"
      aria-hidden="true"
    ></span>
  </div>
</template>
