<script setup lang="ts">
/**
 * A remote desktop, drawn on a canvas.
 *
 * The pixels arrive as changed rectangles over a raw IPC channel; each one is copied into the
 * canvas at its own position, so a keystroke on the far side repaints a few tiles rather than the
 * whole screen. Pointer coordinates are mapped through the canvas's drawn size, which is what makes
 * a desktop that has not been resized yet still usable: the picture is scaled, the coordinates are
 * not guessed.
 *
 * Keyboard input travels as PC scancodes, the same units the far side's keyboard driver speaks.
 * That means a Chinese input method is the *remote* one — the server's own IME — which is also how
 * the system client behaves. Local composition is what the wire type's `text` variant exists for,
 * once the page has an input method story of its own.
 */
import { LoaderCircle, RotateCw } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { registerDesktopSink } from "@/lib/desktop/frames";
import { useTerminalsStore, type DesktopTab } from "@/stores/terminals";
import type { DesktopInput, DesktopMouseButton } from "@/types/session";

const props = defineProps<{
  tab: DesktopTab;
  active: boolean;
  interactive: boolean;
}>();

const { t } = useI18n();
const terminals = useTerminalsStore();

const canvas = ref<HTMLCanvasElement | null>(null);
const surface = ref<HTMLDivElement | null>(null);
const handshake = ref(true);
let context: CanvasRenderingContext2D | null = null;
let unregister: (() => void) | null = null;
let resizing: ReturnType<typeof setTimeout> | null = null;
let lastSize = { width: 0, height: 0 };
let queuedMove: { x: number; y: number } | null = null;
let moveFrame = 0;

const failure = computed(() => (props.tab.failure ? describeShellError(t, props.tab.failure) : null));
const closed = computed(() => props.tab.status === "ended" || props.tab.status === "failed");

/** Sizes the canvas backing store to the desktop the server agreed to. */
function fitCanvas(): void {
  const canvasElement = canvas.value;
  if (!canvasElement) {
    return;
  }
  const width = Math.max(1, props.tab.width);
  const height = Math.max(1, props.tab.height);
  if (canvasElement.width !== width || canvasElement.height !== height) {
    canvasElement.width = width;
    canvasElement.height = height;
    context = canvasElement.getContext("2d", { alpha: false });
    context?.clearRect(0, 0, width, height);
  }
}

/** The desktop coordinates of a pointer event, through however the canvas is being scaled. */
function desktopPoint(event: MouseEvent): { x: number; y: number } | null {
  const canvasElement = canvas.value;
  if (!canvasElement) {
    return null;
  }
  const rect = canvasElement.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) {
    return null;
  }
  const x = Math.round(((event.clientX - rect.left) / rect.width) * canvasElement.width);
  const y = Math.round(((event.clientY - rect.top) / rect.height) * canvasElement.height);
  return {
    x: Math.min(Math.max(x, 0), canvasElement.width - 1),
    y: Math.min(Math.max(y, 0), canvasElement.height - 1),
  };
}

function send(input: DesktopInput): void {
  if (props.tab.handle && props.interactive) {
    void terminals.desktopInput(props.tab.handle, input);
  }
}

function onPointerMove(event: PointerEvent): void {
  const point = desktopPoint(event);
  if (!point || !props.interactive) {
    return;
  }
  queuedMove = point;
  if (moveFrame === 0) {
    moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      const next = queuedMove;
      queuedMove = null;
      if (next) {
        send({ kind: "pointerMove", x: next.x, y: next.y });
      }
    });
  }
}

function buttonOf(event: MouseEvent): DesktopMouseButton | null {
  switch (event.button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    case 3:
      return "back";
    case 4:
      return "forward";
    default:
      return null;
  }
}

function onPointerDown(event: PointerEvent): void {
  if (!props.interactive) {
    return;
  }
  canvas.value?.focus();
  const button = buttonOf(event);
  const point = desktopPoint(event);
  if (!button || !point) {
    return;
  }
  canvas.value?.setPointerCapture(event.pointerId);
  send({ kind: "pointerMove", x: point.x, y: point.y });
  send({ kind: "pointerButton", button, pressed: true });
}

function onPointerUp(event: PointerEvent): void {
  if (!props.interactive) {
    return;
  }
  const button = buttonOf(event);
  const point = desktopPoint(event);
  if (!button || !point) {
    return;
  }
  send({ kind: "pointerMove", x: point.x, y: point.y });
  send({ kind: "pointerButton", button, pressed: false });
  canvas.value?.releasePointerCapture(event.pointerId);
}

function onWheel(event: WheelEvent): void {
  if (!props.interactive) {
    return;
  }
  event.preventDefault();
  const units = (event.deltaY === 0 ? event.deltaX : event.deltaY) > 0 ? 120 : -120;
  send({ kind: "wheel", vertical: event.deltaY !== 0, units });
}

/**
 * PC scancodes for the keys a browser names by *position* rather than by character.
 *
 * `KeyboardEvent.code` is the physical key, which is exactly what the far side's keyboard driver
 * wants: a US-layout letter key and the same key under a Chinese layout are one scancode.
 */
const SCANCODES: Readonly<Record<string, number>> = {
  Escape: 0x01,
  Digit1: 0x02,
  Digit2: 0x03,
  Digit3: 0x04,
  Digit4: 0x05,
  Digit5: 0x06,
  Digit6: 0x07,
  Digit7: 0x08,
  Digit8: 0x09,
  Digit9: 0x0a,
  Digit0: 0x0b,
  Minus: 0x0c,
  Equal: 0x0d,
  Backspace: 0x0e,
  Tab: 0x0f,
  KeyQ: 0x10,
  KeyW: 0x11,
  KeyE: 0x12,
  KeyR: 0x13,
  KeyT: 0x14,
  KeyY: 0x15,
  KeyU: 0x16,
  KeyI: 0x17,
  KeyO: 0x18,
  KeyP: 0x19,
  BracketLeft: 0x1a,
  BracketRight: 0x1b,
  Enter: 0x1c,
  ControlLeft: 0x1d,
  KeyA: 0x1e,
  KeyS: 0x1f,
  KeyD: 0x20,
  KeyF: 0x21,
  KeyG: 0x22,
  KeyH: 0x23,
  KeyJ: 0x24,
  KeyK: 0x25,
  KeyL: 0x26,
  Semicolon: 0x27,
  Quote: 0x28,
  Backquote: 0x29,
  ShiftLeft: 0x2a,
  Backslash: 0x2b,
  KeyZ: 0x2c,
  KeyX: 0x2d,
  KeyC: 0x2e,
  KeyV: 0x2f,
  KeyB: 0x30,
  KeyN: 0x31,
  KeyM: 0x32,
  Comma: 0x33,
  Period: 0x34,
  Slash: 0x35,
  ShiftRight: 0x36,
  NumpadMultiply: 0x37,
  AltLeft: 0x38,
  Space: 0x39,
  CapsLock: 0x3a,
  F1: 0x3b,
  F2: 0x3c,
  F3: 0x3d,
  F4: 0x3e,
  F5: 0x3f,
  F6: 0x40,
  F7: 0x41,
  F8: 0x42,
  F9: 0x43,
  F10: 0x44,
  Pause: 0x45,
  ScrollLock: 0x46,
  Numpad7: 0x47,
  Numpad8: 0x48,
  Numpad9: 0x49,
  NumpadSubtract: 0x4a,
  Numpad4: 0x4b,
  Numpad5: 0x4c,
  Numpad6: 0x4d,
  NumpadAdd: 0x4e,
  Numpad1: 0x4f,
  Numpad2: 0x50,
  Numpad3: 0x51,
  Numpad0: 0x52,
  NumpadDecimal: 0x53,
  F11: 0x57,
  F12: 0x58,
  NumpadEnter: 0xe01c,
  ControlRight: 0xe01d,
  NumpadDivide: 0xe035,
  PrintScreen: 0xe037,
  AltRight: 0xe038,
  Home: 0xe047,
  ArrowUp: 0xe048,
  PageUp: 0xe049,
  ArrowLeft: 0xe04b,
  ArrowRight: 0xe04d,
  End: 0xe04f,
  ArrowDown: 0xe050,
  PageDown: 0xe051,
  Insert: 0xe052,
  Delete: 0xe053,
  MetaLeft: 0xe05b,
  MetaRight: 0xe05c,
  ContextMenu: 0xe05d,
};

function onKey(event: KeyboardEvent, pressed: boolean): void {
  if (!props.interactive || event.isComposing || event.keyCode === 229) {
    return;
  }
  const scancode = SCANCODES[event.code];
  if (scancode === undefined) {
    return;
  }
  event.preventDefault();
  // The transport ignores a press for a key it already thinks is down, which is what makes a held
  // key repeat silently. A repeat is therefore a release followed by a press.
  if (pressed && event.repeat) {
    send({ kind: "key", scancode, pressed: false });
  }
  send({ kind: "key", scancode, pressed });
}

/** Sends the container's size, once the person stops dragging the window around. */
function scheduleResize(): void {
  if (resizing) {
    clearTimeout(resizing);
  }
  resizing = setTimeout(() => {
    resizing = null;
    const element = surface.value;
    const handle = props.tab.handle;
    if (!element || !handle || !props.active) {
      return;
    }
    const width = Math.round(element.clientWidth);
    const height = Math.round(element.clientHeight);
    if (width < 320 || height < 200) {
      return;
    }
    if (width === lastSize.width && height === lastSize.height) {
      return;
    }
    lastSize = { width, height };
    void terminals.desktopResize(handle, width, height);
  }, 300);
}

let observer: ResizeObserver | null = null;

onMounted(() => {
  fitCanvas();
  if (props.tab.handle) {
    unregister = registerDesktopSink(props.tab.handle, (frame) => {
      drawFromFrame(frame);
    });
    // Anything drawn before this pane existed was dropped with the frames that carried it. A
    // full repaint is the one request that recovers the whole desktop, wallpaper included.
    void terminals.desktopRefresh(props.tab.handle);
  }
  observer = new ResizeObserver(() => scheduleResize());
  if (surface.value) {
    observer.observe(surface.value);
  }
  scheduleResize();
});

onBeforeUnmount(() => {
  unregister?.();
  observer?.disconnect();
  if (resizing) {
    clearTimeout(resizing);
  }
  if (moveFrame !== 0) {
    cancelAnimationFrame(moveFrame);
  }
});

// The pane already has a parsed frame from the bus; `draw` is the raw-buffer path the channel uses.
function drawFromFrame(frame: {
  x: number;
  y: number;
  width: number;
  height: number;
  pixels: Uint8ClampedArray<ArrayBuffer>;
}): void {
  const ctx = context;
  if (!ctx) {
    return;
  }
  ctx.putImageData(new ImageData(frame.pixels, frame.width, frame.height), frame.x, frame.y);
  handshake.value = false;
}

watch(
  () => [props.tab.width, props.tab.height],
  () => fitCanvas(),
);
watch(
  () => props.active,
  (active) => {
    if (active) {
      scheduleResize();
    }
  },
);

function reconnect(): void {
  void terminals.reconnect(props.tab.id);
}
</script>

<template>
  <div ref="surface" class="relative flex size-full items-center justify-center overflow-hidden bg-black">
    <canvas
      ref="canvas"
      data-desktop-canvas
      tabindex="0"
      class="max-h-full max-w-full outline-none"
      :style="{ aspectRatio: `${tab.width} / ${tab.height}`, cursor: 'default' }"
      @pointermove="onPointerMove"
      @pointerdown="onPointerDown"
      @pointerup="onPointerUp"
      @wheel="onWheel"
      @keydown="onKey($event, true)"
      @keyup="onKey($event, false)"
      @contextmenu.prevent
    />

    <div v-if="tab.status === 'open' && handshake" class="pointer-events-none absolute inset-0 grid place-items-center">
      <LoaderCircle :size="22" class="animate-spin text-white/70" />
    </div>

    <div
      v-if="closed"
      class="absolute inset-x-0 bottom-0 flex items-center justify-between gap-3 border-t border-white/10 bg-black/70 px-3 py-2"
    >
      <p class="min-w-0 text-[11px] text-white/80">
        <span class="block truncate">{{ failure?.message ?? t("desktop.ended") }}</span>
        <span v-if="failure" class="block font-mono text-[10px] text-white/50">{{ failure.code }}</span>
      </p>
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-white/20 px-2 py-1 text-[11px] text-white/90 transition-colors hover:bg-white/10"
        @click="reconnect"
      >
        <RotateCw :size="11" />
        {{ t("desktop.reconnect") }}
      </button>
    </div>
  </div>
</template>
