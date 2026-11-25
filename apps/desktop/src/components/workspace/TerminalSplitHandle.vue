<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { cn } from "@/lib/utils";
import { type TerminalDivider, TERMINAL_DIVIDER_PX } from "@/lib/workspace/terminal-layout";

const props = defineProps<{ divider: TerminalDivider }>();
const emit = defineEmits<{ move: [ratio: number] }>();
const { t } = useI18n();
const handle = ref<HTMLElement | null>(null);
const dragging = ref(false);
const vertical = computed(() => props.divider.direction === "right");
const style = computed(() => {
  const rect = props.divider.rect;
  return { left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px`, height: `${rect.height}px` };
});

function move(ratio: number): void {
  emit("move", Math.max(props.divider.minRatio, Math.min(props.divider.maxRatio, ratio)));
}

function start(event: PointerEvent): void {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  handle.value?.setPointerCapture(event.pointerId);
  dragging.value = true;
}

function drag(event: PointerEvent): void {
  const parent = handle.value?.parentElement;
  if (!dragging.value || !parent) {
    return;
  }
  const bounds = parent.getBoundingClientRect();
  const region = props.divider.region;
  const available = (vertical.value ? region.width : region.height) - TERMINAL_DIVIDER_PX;
  if (available > 0) {
    const distance = vertical.value
      ? event.clientX - bounds.left - region.left
      : event.clientY - bounds.top - region.top;
    move(distance / available);
  }
}

function finish(event: PointerEvent): void {
  if (!dragging.value) {
    return;
  }
  dragging.value = false;
  if (handle.value?.hasPointerCapture(event.pointerId)) {
    handle.value.releasePointerCapture(event.pointerId);
  }
}

function keydown(event: KeyboardEvent): void {
  const available = (vertical.value ? props.divider.region.width : props.divider.region.height) - TERMINAL_DIVIDER_PX;
  const step = available > 0 ? 16 / available : 0;
  if (event.key === "Home") {
    move(props.divider.minRatio);
  } else if (event.key === "End") {
    move(props.divider.maxRatio);
  } else if (event.key === (vertical.value ? "ArrowLeft" : "ArrowUp")) {
    move(props.divider.ratio - step);
  } else if (event.key === (vertical.value ? "ArrowRight" : "ArrowDown")) {
    move(props.divider.ratio + step);
  } else {
    return;
  }
  event.preventDefault();
}
</script>

<template>
  <div
    ref="handle"
    data-terminal-divider
    :data-split-id="divider.id"
    :style="style"
    :class="
      cn(
        'group absolute z-10 touch-none cursor-default select-none outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
        vertical ? 'cursor-col-resize' : 'cursor-row-resize',
      )
    "
    role="separator"
    tabindex="0"
    :aria-label="t('terminal.split.divider')"
    :aria-orientation="vertical ? 'vertical' : 'horizontal'"
    :aria-valuemin="Math.round(divider.minRatio * 100)"
    :aria-valuemax="Math.round(divider.maxRatio * 100)"
    :aria-valuenow="Math.round(divider.ratio * 100)"
    @pointerdown="start"
    @pointermove="drag"
    @pointerup="finish"
    @pointercancel="finish"
    @lostpointercapture="dragging = false"
    @keydown="keydown"
    @dblclick="move(0.5)"
  >
    <span
      :class="
        cn(
          'pointer-events-none absolute bg-border transition-colors',
          dragging ? 'bg-primary' : 'group-hover:bg-primary/60',
          vertical ? 'inset-y-0 left-1/2 w-px' : 'inset-x-0 top-1/2 h-px',
        )
      "
    />
    <span
      class="panel-grip"
      :class="[!vertical && 'panel-grip--row', dragging ? 'text-primary' : 'text-muted-foreground/50']"
      aria-hidden="true"
    />
  </div>
</template>
