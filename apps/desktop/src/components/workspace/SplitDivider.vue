<script setup lang="ts">
/**
 * The line between the terminal and the file panel, and the drag that moves it.
 *
 * It owns no size of its own: it reports where the line was put, and whoever owns the layout decides
 * what that means. What it does own are the bounds, and the fact that the drag has to be bounded by
 * the room the two panes are inside — which is why it watches the element it sits in rather than
 * being told a number that the window could invalidate a moment later.
 *
 * The same bounds are applied to what is drawn: the pane below carries a minimum height, so a
 * height that no longer fits the window shrinks the panel rather than squeezing the terminal to
 * nothing. Two places, one rule, and the rule is in `lib/workspace/split.ts`.
 *
 * Both a pointer and a keyboard can move it. A separator that only answers to a mouse is a control
 * half the people cannot use, and this one is a control like any other: arrows nudge it, Home and
 * End take it to the stops, and it says where it is.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { MIN_PANE_PX, SPLIT_DIVIDER_PX, clampPaneHeight } from "@/lib/workspace/split";
import { cn } from "@/lib/utils";

const props = defineProps<{
  /** Where the line is now: the height of the pane below it, in pixels. */
  height: number;
}>();

const emit = defineEmits<{
  /** The line moved. Fires for every pointer move of a drag. */
  move: [height: number];
  /** The line was let go, or nudged with a key: where it ended up is worth writing down. */
  settle: [];
  /** The line was double-clicked: back to where it started. */
  reset: [];
}>();

const { t } = useI18n();
const divider = ref<HTMLDivElement | null>(null);
const dragging = ref(false);

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
function bounded(height: number): number {
  return clampPaneHeight(height, available.value, MIN_PANE_PX);
}

/** The topmost the line may be dragged to; what the room costs is the floor pane and this line. */
const ceiling = computed(() => Math.max(MIN_PANE_PX, available.value - MIN_PANE_PX - SPLIT_DIVIDER_PX));

function moveTo(height: number): void {
  emit("move", bounded(height));
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
  // Measured from the bottom of the region rather than tracked as a delta: the pane below is the
  // one being sized, so its height is its distance from the floor.
  moveTo(region.getBoundingClientRect().bottom - event.clientY);
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
    case "ArrowUp":
      moveTo(props.height + STEP_PX);
      break;
    case "ArrowDown":
      moveTo(props.height - STEP_PX);
      break;
    case "Home":
      moveTo(MIN_PANE_PX);
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
  available.value = region.clientHeight;
  observer = new ResizeObserver(() => {
    available.value = region.clientHeight;
  });
  observer.observe(region);
});

onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div
    ref="divider"
    :class="
      cn(
        'group relative h-2 shrink-0 cursor-row-resize touch-none select-none outline-none',
        'focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
      )
    "
    role="separator"
    aria-orientation="horizontal"
    tabindex="0"
    :aria-label="t('terminal.panel.divider')"
    :aria-valuemin="MIN_PANE_PX"
    :aria-valuemax="Math.round(ceiling)"
    :aria-valuenow="Math.round(height)"
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
          'pointer-events-none absolute inset-x-0 top-1/2 -translate-y-1/2 bg-border transition-all',
          dragging ? 'h-0.5 bg-primary' : 'h-px group-hover:h-0.5 group-hover:bg-primary/60',
        )
      "
    />
    <!--
      Three dots in the middle, lying along the line: the mark that says this edge can be moved.
      Without it the divider is a line that only shows itself to somebody whose pointer happens to
      cross it, which is a control nobody finds by looking.
    -->
    <span
      class="panel-grip panel-grip--row"
      :class="dragging ? 'text-primary' : 'text-muted-foreground/50 group-hover:text-primary/70'"
      aria-hidden="true"
    ></span>
  </div>
</template>
