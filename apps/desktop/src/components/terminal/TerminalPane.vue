<script setup lang="ts">
/**
 * One xterm instance, drawing one handle.
 *
 * Output arrives as bytes and is written as bytes. Decoding base64 into a string on the way in is
 * the one thing that must not happen here: a UTF-8 character split across two events would decode
 * to replacement characters on both sides, and the terminal would show them instead of the
 * character the server sent. Input travels the other way as UTF-8 bytes, which is what `onData`
 * hands over.
 *
 * The size follows the container. A terminal whose size disagrees with the channel's wraps in the
 * wrong place, so the first fit after mounting is sent unconditionally; later ones are debounced,
 * because a window drag produces a resize storm and each one is a network round trip.
 */
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { type ITheme, Terminal } from "@xterm/xterm";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

import { registerTerminalSink } from "@/lib/terminal/bus";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";

import "@xterm/xterm/css/xterm.css";

const preferences = usePreferencesStore();

/** Which of the two palettes this pane wears, from the person's choice rather than from the window. */
const paletteClass = computed(() =>
  preferences.resolvedTerminalTheme === "dark" ? "terminal-dark" : "terminal-light",
);

const props = defineProps<{
  handle: string;
  /** True for the tab on screen: the only one that fits itself and takes focus. */
  active: boolean;
  /** False once the terminal has ended, when keystrokes have nowhere left to go. */
  interactive: boolean;
}>();

const terminals = useTerminalsStore();
const container = ref<HTMLDivElement | null>(null);

const encoder = new TextEncoder();
const FIT_DEBOUNCE_MS = 60;

let terminal: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let unregister: (() => void) | undefined;
let observer: ResizeObserver | undefined;
let resizeTimer: ReturnType<typeof setTimeout> | undefined;
/** The last size the backend was told, so a debounced fit that changed nothing says nothing. */
let announced = { columns: 0, rows: 0 };
let probe: CanvasRenderingContext2D | null | undefined;

/**
 * The terminal palette is the one in the stylesheet, like every other colour in the interface.
 * xterm parses the strings it is given and does not understand `oklch()`, so each token is handed
 * to the browser first and the value it reports back — an sRGB one — is what xterm receives.
 */
function token(name: string, element: Element | null): string | undefined {
  const raw = getComputedStyle(element ?? document.documentElement)
    .getPropertyValue(name)
    .trim();
  if (raw === "" || typeof CSS === "undefined" || !CSS.supports("color", raw)) {
    return undefined;
  }
  if (probe === undefined) {
    probe = document.createElement("canvas").getContext("2d");
  }
  if (!probe) {
    return raw;
  }
  probe.fillStyle = raw;
  return probe.fillStyle;
}

/// The colours the terminal reads. Every one of them is required: an unset colour shows up as
/// black on black, which reads as a rendering bug rather than a missing token.
const TERMINAL_TOKENS = [
  ["background", "--terminal-background"],
  ["foreground", "--terminal-foreground"],
  ["cursor", "--terminal-cursor"],
  ["selectionBackground", "--terminal-selection"],
  ["black", "--terminal-black"],
  ["red", "--terminal-red"],
  ["green", "--terminal-green"],
  ["yellow", "--terminal-yellow"],
  ["blue", "--terminal-blue"],
  ["magenta", "--terminal-magenta"],
  ["cyan", "--terminal-cyan"],
  ["white", "--terminal-white"],
  ["brightBlack", "--terminal-bright-black"],
  ["brightRed", "--terminal-bright-red"],
  ["brightGreen", "--terminal-bright-green"],
  ["brightYellow", "--terminal-bright-yellow"],
  ["brightBlue", "--terminal-bright-blue"],
  ["brightMagenta", "--terminal-bright-magenta"],
  ["brightCyan", "--terminal-bright-cyan"],
  ["brightWhite", "--terminal-bright-white"],
] as const;

function terminalTheme(): ITheme {
  const theme: Record<string, string | undefined> = {};

  for (const [name, customProperty] of TERMINAL_TOKENS) {
    // Read from the pane, not the document: the pane carries the palette class, and a dark
    // terminal in a light interface is a choice somebody is allowed to make.
    theme[name] = token(customProperty, container.value);
  }

  return theme as ITheme;
}

function monospaceFamily(): string | undefined {
  const value = getComputedStyle(document.documentElement).getPropertyValue("--font-mono").trim();
  return value === "" ? undefined : value;
}

function applySize(): void {
  const instance = terminal;
  const fit = fitAddon;
  const element = container.value;
  if (!instance || !fit || !element) {
    return;
  }
  // A pane that is not on screen measures zero: fitting it would ask the server for a terminal
  // with no columns, and the next real fit would have to undo it.
  if (element.clientWidth === 0 || element.clientHeight === 0) {
    return;
  }

  try {
    fit.fit();
  } catch {
    // A pane caught mid-transition can still be too small to fit. Nothing is wrong with the
    // terminal, and the next resize tries again — this must not take the observer down with it.
    return;
  }
  const { cols, rows } = instance;
  if (cols < 1 || rows < 1 || (cols === announced.columns && rows === announced.rows)) {
    return;
  }
  announced = { columns: cols, rows };
  void terminals.resize(props.handle, cols, rows);
}

function scheduleFit(): void {
  if (resizeTimer !== undefined) {
    clearTimeout(resizeTimer);
  }
  resizeTimer = setTimeout(() => {
    resizeTimer = undefined;
    applySize();
  }, FIT_DEBOUNCE_MS);
}

onMounted(() => {
  const element = container.value;
  if (!element) {
    return;
  }

  const instance = new Terminal({
    cursorBlink: true,
    fontFamily: monospaceFamily(),
    fontSize: 13,
    scrollback: 5000,
    theme: terminalTheme(),
  });
  const fit = new FitAddon();
  instance.loadAddon(fit);
  instance.loadAddon(new WebLinksAddon());
  instance.open(element);

  terminal = instance;
  fitAddon = fit;
  unregister = registerTerminalSink(props.handle, (bytes) => instance.write(bytes));
  instance.onData((data) => {
    if (!props.interactive) {
      return;
    }
    void terminals.send(props.handle, encoder.encode(data));
  });

  observer = new ResizeObserver(() => scheduleFit());
  observer.observe(element);

  applySize();
  if (props.active) {
    instance.focus();
  }
});

watch(
  () => preferences.resolvedTerminalTheme,
  () => {
    // The palette lives in the stylesheet, so the tokens have to be read again after the class
    // has changed — which is why this waits a tick instead of reading them straight away.
    void nextTick(() => {
      if (terminal) {
        terminal.options.theme = terminalTheme();
      }
    });
  },
);

watch(
  () => props.active,
  (active) => {
    if (!active) {
      return;
    }
    // Waited for the pane to be laid out: it had no size while it was hidden.
    void nextTick(() => {
      applySize();
      terminal?.focus();
    });
  },
);

onBeforeUnmount(() => {
  if (resizeTimer !== undefined) {
    clearTimeout(resizeTimer);
  }
  observer?.disconnect();
  unregister?.();
  terminal?.dispose();
});
</script>

<template>
  <div
    ref="container"
    class="min-h-0 min-w-0 flex-1 overflow-hidden bg-[var(--terminal-background)]"
    :class="paletteClass"
  ></div>
</template>
