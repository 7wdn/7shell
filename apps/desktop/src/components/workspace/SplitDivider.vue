<script setup lang="ts">
/**
 * The line between two panes, and the drag that moves it — across a stacked pair or down a
 * side-by-side one.
 *
 * It owns no size of its own: it reports where the line was put, and whoever owns the layout decides
 * what that means. What it does own are the bounds, and the fact that the drag has to be bounded by
 * the room the two panes are inside — which is why it watches the element it sits in rather than
 * being told a number that the window could invalidate a moment later.
 *
 * The same bounds are applied to what is drawn: the sized pane carries a floor, so a size that no
 * longer fits the window shrinks the panel rather than squeezing the other side to nothing. Two
 * places, one rule, and the rule is in `lib/workspace/split.ts`.
 *
 * Both a pointer and a keyboard can move it. A separator that only answers to a mouse is a control
 * half the people cannot use, and this one is a control like any other: arrows nudge it, Home and
 * End take it to the stops, and it says where it is.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import {
  MIN_PANE_PX,
  SIDE_DIVIDER_PX,
  SIDE_PANE_KEEP_PX,
  SPLIT_DIVIDER_PX,
  clampPaneHeight,
  clampPaneWidth,
} from "@/lib/workspace/split";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    /** The size of the pane being sized, in pixels: its height or its width. */
    size: number;
    /** Which way the line runs: across a stacked pair, or down a side-by-side pair. */
    orientation?: "horizontal" | "vertical";
    /** Which side of the line the sized pane is on. */
    pane?: "start" | "end";
    /** The least the sized pane may be drawn at. */
    minSize?: number;
  }>(),
  { orientation: "horizontal", pane: "end", minSize: MIN_PANE_PX },
);

const emit = defineEmits<{
  /** The line moved. Fires for every pointer move of a drag. */
  move: [size: number];
  /** The line was let go, or nudged with a key: where it ended up is worth writing down. */
  settle: [];
  /** The line was double-clicked: back to where it started. */
  reset: [];
}>();

const { t } = useI18n();
const divider = ref<HTMLDivElement | null>(null);
const dragging = ref(false);
const vertical = computed(() => props.orientation === "vertical");

/**
 * How much room the two panes and this line have between them.
 *
 * Watched rather than measured once, because the number is read during a render as well as during
 * a drag: the window can be resized between the two, and `aria-valuemax` should not outlive the
 * shape it described. The divider is a direct child of the region it splits — the element it is
 * inside is exactly the two panes plus itself.
 */
const available = ref(0);
let observer: ResizeObserver | undefined;

/** One press of an arrow key: about a line of text, so the movement is legible as it happens. */
const STEP_PX = 16;

/** Where the line may be put, given the room there is now. */
function bounded(size: number): number {
  return vertical.value
    ? clampPaneWidth(size, available.value, props.minSize)
    : clampPaneHeight(size, available.value, props.minSize);
}

/** The topmost the line may be dragged to; the other pane and this line keep what they need. */
const ceiling = computed(() => {
  if (vertical.value) {
    return Math.max(props.minSize, available.value - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX);
  }
  return Math.max(props.minSize, available.value - MIN_PANE_PX - SPLIT_DIVIDER_PX);
});

function moveTo(size: number): void {
  emit("move", bounded(size));
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) {
    return;
  }
  // A drag across a terminal is a drag, not a selection: without this the terminal takes the
  // pointer and starts selecting the text underneath.
  event.preventDefault();
  divider.value?.setPointerCapture(event.pointerId);
  dragging.value = true;
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  const region = divider.value?.parentElement;
  if (!region) {
    return;
  }
  const bounds = region.getBoundingClientRect();
  // Measured from the edge the sized pane sits against rather than tracked as a delta: the pane
  // being sized is the distance between that edge and the line.
  if (vertical.value) {
    moveTo(props.pane === "start" ? event.clientX - bounds.left : bounds.right - event.clientX);
  } else {
    moveTo(props.pane === "start" ? event.clientY - bounds.top : bounds.bottom - event.clientY);
  }
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
  const start = props.pane === "start";
  switch (event.key) {
    case "ArrowUp":
      if (vertical.value) return;
      moveTo(props.size + (start ? -STEP_PX : STEP_PX));
      break;
    case "ArrowDown":
      if (vertical.value) return;
      moveTo(props.size + (start ? STEP_PX : -STEP_PX));
      break;
    case "ArrowLeft":
      if (!vertical.value) return;
      moveTo(props.size + (start ? -STEP_PX : STEP_PX));
      break;
    case "ArrowRight":
      if (!vertical.value) return;
      moveTo(props.size + (start ? STEP_PX : -STEP_PX));
      break;
    case "Home":
      moveTo(props.minSize);
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
  const region = divider.value?.parentElement;
  if (!region) {
    return;
  }
  const measure = () => {
    available.value = vertical.value ? region.clientWidth : region.clientHeight;
  };
  measure();
  observer = new ResizeObserver(measure);
  observer.observe(region);
});

onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div
    ref="divider"
    data-split-divider
    :class="
      cn(
        'group relative shrink-0 touch-none select-none outline-none',
        'focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
        vertical ? 'w-2 cursor-col-resize' : 'h-2 cursor-row-resize',
      )
    "
    role="separator"
    :aria-orientation="vertical ? 'vertical' : 'horizontal'"
    tabindex="0"
    :aria-label="t('terminal.panel.divider')"
    :aria-valuemin="props.minSize"
    :aria-valuemax="Math.round(ceiling)"
    :aria-valuenow="Math.round(size)"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeydown"
    @dblclick="emit('reset')"
  >
    <!--
      The line is drawn on hover and while dragging, and is always there to be grabbed: it is a
      target rather than a decoration, so what changes is how visible it is, not whether it exists.
    -->
    <span
      :class="
        cn(
          'pointer-events-none absolute bg-border transition-all',
          dragging ? 'bg-primary' : 'group-hover:bg-primary/60',
          vertical
            ? cn('inset-y-0 left-1/2 -translate-x-1/2', dragging ? 'w-0.5' : 'w-px group-hover:w-0.5')
            : cn('inset-x-0 top-1/2 -translate-y-1/2', dragging ? 'h-0.5' : 'h-px group-hover:h-0.5'),
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
      :class="[
        !vertical && 'panel-grip--row',
        dragging ? 'text-primary' : 'text-muted-foreground/50 group-hover:text-primary/70',
      ]"
      aria-hidden="true"
    ></span>
  </div>
</template>
