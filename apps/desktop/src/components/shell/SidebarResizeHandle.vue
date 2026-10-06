<script setup lang="ts">
/**
 * The edge between the session list and the workspace, and the drag that moves it.
 *
 * It owns no width of its own: it reports where the edge was put, and the sidebar — which is what
 * the width is about — decides what that means and writes it down. What this owns is the bounds and
 * the fact that the drag has to be bounded by the room the two panels are inside, which is why it
 * watches the element they sit in rather than being told a number the window could invalidate a
 * moment later.
 *
 * It carries a mark: three dots in the middle of the edge. An edge that can be moved has to say so,
 * because the alternative — a line that appears when the pointer happens to be over it — is a
 * control only the people who already know about it can find.
 *
 * Both a pointer and a keyboard can move it. A separator that only answers to a mouse is a control
 * half the people cannot use, and this one is a control like any other: arrows nudge it, Home and
 * End take it to the stops, and it says where it is.
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  /** How wide the panel is now, in pixels. */
  width: number;
}>();

const emit = defineEmits<{
  /** The edge moved. Fires for every pointer move of a drag. */
  move: [width: number];
  /** The edge was let go, or nudged with a key: where it ended up is worth writing down. */
  settle: [];
  /** The edge was double-clicked: back to where it started. */
  reset: [];
}>();

/**
 * How narrow and how wide the panel may be.
 *
 * The floor is where a session row stops fitting its own name and its buttons; the ceiling is where
 * the workspace would start losing to the list, and it is applied against the window rather than
 * fixed, because a laptop and a monitor do not have the same room.
 */
const MIN_WIDTH = 240;
const MAX_WIDTH = 800;
/** What the workspace keeps for itself, whatever the panel would like. */
const WORKSPACE_KEEP = 360;
/** One press of an arrow key: about a line of text, so the movement is legible as it happens. */
const STEP = 16;

const { t } = useI18n();
const handle = ref<HTMLDivElement | null>(null);
const dragging = ref(false);

/** How much room the panel and the workspace have between them. */
const available = ref(0);
let observer: ResizeObserver | undefined;

/** The widest the panel may be, given the room there is now. */
const ceiling = computed(() => Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, available.value - WORKSPACE_KEEP)));

function bounded(width: number): number {
  return Math.max(MIN_WIDTH, Math.min(ceiling.value, Math.round(width)));
}

function moveTo(width: number): void {
  emit("move", bounded(width));
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) {
    return;
  }
  // A drag across the panel is a drag, not a selection: without this the list selects its own text
  // as the pointer sweeps over it.
  event.preventDefault();
  handle.value?.setPointerCapture(event.pointerId);
  dragging.value = true;
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value || !handle.value) {
    return;
  }
  // Measured from the left of the panel rather than tracked as a delta: the pointer is over the
  // edge, so the edge is where the pointer is.
  const left = handle.value.parentElement?.getBoundingClientRect().left ?? 0;
  moveTo(event.clientX - left);
}

function onPointerUp(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  dragging.value = false;
  handle.value?.releasePointerCapture(event.pointerId);
  emit("settle");
}

function onKeydown(event: KeyboardEvent): void {
  switch (event.key) {
    case "ArrowLeft":
      moveTo(props.width - STEP);
      break;
    case "ArrowRight":
      moveTo(props.width + STEP);
      break;
    case "Home":
      moveTo(MIN_WIDTH);
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
  // The panel is inside the row it shares with the workspace, and that row is the room there is.
  const row = handle.value?.parentElement?.parentElement;
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
    ref="handle"
    data-sidebar-divider
    class="group absolute inset-y-0 -right-1 z-20 w-2 cursor-col-resize touch-none select-none outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
    role="separator"
    aria-orientation="vertical"
    tabindex="0"
    :aria-label="t('shell.sidebar.divider')"
    :aria-valuemin="MIN_WIDTH"
    :aria-valuemax="Math.round(ceiling)"
    :aria-valuenow="Math.round(props.width)"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeydown"
    @dblclick="emit('reset')"
  >
    <span
      class="panel-grip"
      :class="dragging ? 'text-primary' : 'text-muted-foreground/45 group-hover:text-primary/70'"
      aria-hidden="true"
    ></span>
  </div>
</template>
