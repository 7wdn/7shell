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
 *
 * Input is read on the way past for one purpose: a `cd` is the only thing that says where the shell
 * is on a host that does not report it, and the file panel under this terminal follows it. The line
 * is read and never held — dropped at every Enter — and a shell that reports its own directory
 * (OSC 7) is followed exactly instead of by reading anything.
 *
 * The right-click belongs to the shell's menu rather than to xterm. It is taken in the capture phase,
 * which is the only way to get there first: xterm listens for the same event on the element it draws
 * into, and on a Mac it uses it to select the word under the pointer — which would throw away the
 * selection somebody had already made, immediately before copying it.
 *
 * A selection also raises a small toolbar of its own — copy, hand it to the shell, find — so the
 * three things a person does with a highlight are reachable without a right-click. Where it stands
 * is read from the renderer's own selection boxes; `lib/terminal/selection.ts` carries that story.
 */
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { type ITheme, Terminal } from "@xterm/xterm";
import { ChevronDown, ChevronUp, X } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useContextMenu } from "@/composables/useContextMenu";
import { terminalMenu, terminalSelectionTools } from "@/lib/menu/terminal";
import { MENU_TITLES } from "@/lib/menu/titles";
import type { MenuAction } from "@/lib/menu/types";
import { matchesShortcut } from "@/lib/shortcuts/binding";
import { formatShortcutDisplay } from "@/lib/shortcuts/display";
import { bindingFor } from "@/lib/shortcuts/registry";
import { registerTerminalSink } from "@/lib/terminal/bus";
import { TypedCommands, directoryFromOsc7 } from "@/lib/terminal/follow";
import { TOOLS_ANCHOR_INSET_PX, TOOLS_GAP_PX, placeSelectionTools, selectionShape } from "@/lib/terminal/selection";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";

import "@xterm/xterm/css/xterm.css";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();

/** Which of the two palettes this pane wears, from the person's choice rather than from the window. */
const paletteClass = computed(() =>
  preferences.resolvedTerminalTheme === "dark" ? "terminal-dark" : "terminal-light",
);

/**
 * Find-in-terminal state.
 *
 * The bar belongs to the pane rather than to the workspace because the thing being searched is the
 * scrollback xterm holds, and only the pane has it. The find key (Ctrl+F by default) opens it, Enter
 * walks forward, Escape closes it and hands the keyboard back to the shell — the terminal is a place
 * where a lost keystroke is a command that did not run.
 */
const searching = ref(false);
const query = ref("");
const caseSensitive = ref(false);
const results = ref<{ index: number; count: number } | null>(null);
const searchInput = ref<HTMLInputElement | null>(null);

let searchAddon: SearchAddon | null = null;

const props = defineProps<{
  handle: string;
  /** True for the tab on screen: the only one that fits itself and takes focus. */
  active: boolean;
  /** False once the terminal has ended, when keystrokes have nowhere left to go. */
  interactive: boolean;
}>();

const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();
const container = ref<HTMLDivElement | null>(null);
/** The pane itself, which is what the selection toolbar is positioned against and clamped inside. */
const paneBox = ref<HTMLDivElement | null>(null);

/** What has been typed since the last Enter, so a `cd` in it can be read for the file panel. */
const typed = new TypedCommands();

const encoder = new TextEncoder();
const FIT_DEBOUNCE_MS = 60;

let terminal: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let unregister: (() => void) | undefined;
let osc7: { dispose: () => void } | undefined;
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
  // A new size is new rows under the selection, which is where the toolbar is standing.
  if (toolsAt.value !== null) {
    void repositionTools();
  }
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

/**
 * What the terminal's two surfaces ask of the pane — its right-click menu and the toolbar over a
 * selection.
 *
 * Built fresh whenever one of them is put together rather than kept in a constant: nothing here has
 * to be told when the terminal behind it comes or goes, so a pane still open on a connection that
 * has ended cannot hand keystrokes to a terminal that is gone.
 */
function terminalActions() {
  return {
    // Through the terminal rather than around it: `paste` is the same path a real paste takes,
    // which is what makes a bracketed paste and a multi-line paste behave the way they should.
    paste: (text: string) => terminal?.paste(text),
    selectAll: () => terminal?.selectAll(),
    clear: () => terminal?.clear(),
    dropSelection: () => terminal?.clearSelection(),
    find: (needle: string) => openSearch(needle),
  };
}

/**
 * The terminal's own right-click menu.
 *
 * The selection is read here rather than inside the menu, because it is a property of the moment the
 * menu was opened: what Copy offers is what was highlighted when somebody right-clicked.
 */
function onContextMenu(event: MouseEvent): void {
  const instance = terminal;
  if (!instance) {
    return;
  }
  openMenu(
    event,
    { key: MENU_TITLES.terminal },
    terminalMenu({ selection: instance.getSelection(), interactive: props.interactive }, terminalActions()),
  );
}

/**
 * Opens the bar, or focuses it when it is already open.
 *
 * A needle — the toolbar's magnifier hands over the selection it was pressed over — is put in the
 * field and searched at once, because the reason that button was pressed is the text underneath it,
 * and a bar that opened empty would be asking somebody to copy by hand what they had just
 * highlighted.
 *
 * Only the first line that says anything is taken: a selection can cover several rows and a find
 * field holds one line. That line is trimmed at both ends and otherwise left alone — what is being
 * looked for is what was highlighted.
 */
function openSearch(needle?: string): void {
  const seeded = needle?.split("\n").find((line) => line.trim() !== "");
  if (seeded !== undefined) {
    query.value = seeded.trim();
  }
  searching.value = true;
  void nextTick(() => {
    searchInput.value?.select();
    if (seeded !== undefined) {
      runSearch();
    }
  });
}

/**
 * Closes it and hands the keyboard back to the shell.
 *
 * The decorations go with it: a highlight left behind is a screen that keeps answering a question
 * nobody is asking any more.
 */
function closeSearch(): void {
  searching.value = false;
  results.value = null;
  searchAddon?.clearDecorations();
  terminal?.focus();
}

/**
 * The colours a found match is drawn in.
 *
 * Read from the pane's own palette, like the terminal's colours are, so a match is the same muted
 * highlight a selection is in either theme. The active one is told apart by an outline rather than by
 * a second background — a distinction that survives a light terminal as well as a dark one.
 *
 * These are asked for by `runSearch` and not only for the drawing: the addon reports how many matches
 * there are — which is the count the bar prints — only when it was given something to draw them
 * with, so the marks and the count are one thing.
 */
function searchDecorations() {
  return {
    matchBackground: token("--terminal-selection", container.value),
    matchOverviewRuler: "#00000000",
    activeMatchBackground: token("--terminal-selection", container.value),
    activeMatchBorder: token("--terminal-yellow", container.value),
    activeMatchColorOverviewRuler: "#00000000",
  };
}

/** Walks the matches, which is what typing and Enter both do. */
function runSearch(backwards = false): void {
  const needle = query.value;
  if (needle === "" || !searchAddon) {
    results.value = null;
    searchAddon?.clearDecorations();
    return;
  }

  const options = { caseSensitive: caseSensitive.value, incremental: true, decorations: searchDecorations() };
  if (backwards) {
    searchAddon.findPrevious(needle, options);
  } else {
    searchAddon.findNext(needle, options);
  }
}

/**
 * The little toolbar over a selection, and where it stands.
 *
 * `null` is the whole of its visibility: it exists while a selection is drawn on screen, and the
 * place it stands is also the flag that it is standing, so the two cannot come apart. Nothing is
 * remembered about a selection that has been handled — there is no reason to keep text the terminal
 * is no longer showing.
 */
const toolsAt = ref<{ left: number; top: number } | null>(null);
const toolsBox = ref<HTMLDivElement | null>(null);
const toolsSelection = ref("");
/** Whether a press began in the terminal and has not been released yet — see `onWindowPointerUp`. */
let pressed = false;

/** The toolbar's entries, rebuilt as the selection under it, or the terminal's liveness, changes. */
const toolEntries = computed(() =>
  terminalSelectionTools({ selection: toolsSelection.value, interactive: props.interactive }, terminalActions()),
);

function hideTools(): void {
  toolsAt.value = null;
}

/**
 * Puts the toolbar against the selection, and says whether any of it is still on screen.
 *
 * A selection the person scrolled away from has no drawn segments left, so there is nothing to
 * stand over: false is where the caller puts the toolbar away. Read from the renderer's own
 * selection boxes rather than computed from cells — `selectionShape` in `lib/terminal/selection.ts`
 * carries why — and read again on every scroll, because the highlight is glued to the rows and not
 * to the pane.
 */
async function positionTools(): Promise<boolean> {
  const surface = container.value;
  const pane = paneBox.value;
  if (surface === null || pane === null) {
    return false;
  }
  const selection = selectionShape(surface);
  if (selection === null) {
    return false;
  }
  const paneRect = pane.getBoundingClientRect();
  const placed = (width: number, height: number) => placeSelectionTools(selection, paneRect, { width, height });

  const drawn = toolsBox.value;
  if (drawn !== null) {
    toolsAt.value = placed(drawn.offsetWidth, drawn.offsetHeight);
    return true;
  }

  // Not drawn yet, so it has no size to be placed with: it is put at the end of the selection and
  // moved once it has been in the document for a tick — a frame nobody sees, and less than a
  // toolbar whose first appearance is somewhere it does not belong.
  toolsAt.value = {
    left: selection.end.right - paneRect.left - TOOLS_ANCHOR_INSET_PX,
    top: selection.end.bottom - paneRect.top + TOOLS_GAP_PX,
  };
  await nextTick();
  const fresh = toolsBox.value;
  if (fresh !== null && toolsAt.value !== null) {
    toolsAt.value = placed(fresh.offsetWidth, fresh.offsetHeight);
  }
  return true;
}

/** Re-aims a toolbar that is already up, and takes it away when there is nothing left to aim at. */
async function repositionTools(): Promise<void> {
  if (!(await positionTools())) {
    hideTools();
  }
}

/**
 * Shows the toolbar over the selection.
 *
 * A frame first, and the same sense of "drawn" the shell's own first-frame report uses: xterm paints
 * the selection layer on an animation frame of its own, so reading the document in the same turn as
 * the release that made the selection would measure the frame before it.
 */
function revealTools(): void {
  void (async () => {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const instance = terminal;
    if (!instance?.hasSelection()) {
      hideTools();
      return;
    }
    toolsSelection.value = instance.getSelection();
    await repositionTools();
  })();
}

/**
 * The press that can begin a selection, and the release that ends one.
 *
 * The release is watched for on the window rather than on the terminal: a drag that reaches the end
 * of a line usually carries the pointer out of the pane, and the selection is made all the same. The
 * flag is what keeps a release that belongs to somebody else — a click on another pane's button —
 * from drawing a toolbar over a selection nobody was just making.
 */
function onSurfacePointerDown(event: PointerEvent): void {
  pressed = event.button === 0;
}

function onWindowPointerUp(): void {
  if (!pressed) {
    return;
  }
  pressed = false;
  revealTools();
}

/**
 * Pressing the toolbar's own buttons is not "going elsewhere": the press that runs one must not take
 * it away before the click lands. Any other press does take it away — the same rule the context
 * menus follow.
 */
function onDocumentPointerDown(event: PointerEvent): void {
  pressed = false;
  const target = event.target;
  if (target instanceof Node && toolsBox.value?.contains(target) === true) {
    return;
  }
  hideTools();
}

/** What a button says when pointed at: its name, and the key of the one action that has a real one. */
function toolsTooltip(entry: MenuAction): string {
  const name = t(entry.labelKey);
  if (entry.shortcut === undefined) {
    return name;
  }
  return `${name} (${formatShortcutDisplay(bindingFor(entry.shortcut, preferences.keybindings), app.platform)})`;
}

/** Runs one, and hands the keyboard back: the toolbar is a menu that happens to be drawn as buttons. */
function runTool(entry: MenuAction): void {
  hideTools();
  terminal?.focus();
  void entry.run();
}

onMounted(() => {
  const element = container.value;
  if (!element) {
    return;
  }

  const instance = new Terminal({
    cursorBlink: true,
    fontFamily: preferences.wornTerminalFont,
    fontSize: preferences.terminalFontSize,
    scrollback: 5000,
    theme: terminalTheme(),
    // The menu that opens on a right-click works on the selection somebody made before opening it,
    // and this would replace that selection with one word.
    rightClickSelectsWord: false,
    // The find addon draws every match with the decoration API, which xterm still keeps behind this
    // switch: without it a search that asks for decorations throws instead of reporting, and the bar
    // loses its marks and its match count with it. Nothing else here reaches for a proposed API.
    allowProposedApi: true,
  });
  const fit = new FitAddon();
  instance.loadAddon(fit);
  instance.loadAddon(new WebLinksAddon());
  const search = new SearchAddon();
  instance.loadAddon(search);
  search.onDidChangeResults(({ resultIndex, resultCount }) => {
    results.value = { index: resultIndex, count: resultCount };
  });
  searchAddon = search;
  // Find-in-terminal's key belongs to the terminal: without this the shell receives it. The key is
  // read from the table rather than spelled out here, so whatever prints it and whatever listens for
  // it cannot end up naming different keys.
  instance.attachCustomKeyEventHandler((event) => {
    if (
      event.type === "keydown" &&
      matchesShortcut(event, bindingFor("terminal.find", preferences.keybindings), app.platform)
    ) {
      openSearch();
      return false;
    }
    return true;
  });
  instance.open(element);

  terminal = instance;
  fitAddon = fit;
  unregister = registerTerminalSink(props.handle, (bytes) => instance.write(bytes));
  instance.onData((data) => {
    if (!props.interactive) {
      return;
    }
    // Read before it leaves: a `cd` typed here is the one thing that says where the shell is on a
    // host that does not report it, and the file panel under this terminal follows it.
    const directory = typed.feed(data);
    if (directory !== null) {
      void terminals.followDirectory(props.handle, directory);
    }
    void terminals.send(props.handle, encoder.encode(data));
  });
  // And the exact report, where the far side is configured to give one. It says nothing about the
  // terminal, so it is passed on either way.
  osc7 = instance.parser.registerOscHandler(7, (data) => {
    const directory = directoryFromOsc7(data);
    if (directory !== null) {
      void terminals.followDirectory(props.handle, directory);
    }
    return false;
  });

  instance.onSelectionChange(() => {
    // The highlight is what the toolbar is about: with nothing selected there is nothing to stand
    // over, whatever else asked for it to be there.
    if (!instance.hasSelection()) {
      hideTools();
    }
  });
  instance.onScroll(() => {
    // The rows move under the highlight, so a toolbar left where it was would point at something
    // else. Followed rather than dismissed: scrolling is somebody looking around, not walking away.
    if (toolsAt.value !== null) {
      void repositionTools();
    }
  });

  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  window.addEventListener("pointerup", onWindowPointerUp);

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
  () => preferences.terminalFontSize,
  () => {
    // The size is what the grid is measured in, so a new one is a new number of columns and rows —
    // which is why this hands the size over and then fits, in that order.
    void nextTick(() => {
      if (terminal) {
        terminal.options.fontSize = preferences.terminalFontSize;
        applySize();
      }
    });
  },
);

watch(
  () => preferences.wornTerminalFont,
  () => {
    // A terminal is built with its font and does not go looking for a new one, so what has changed
    // has to be handed to it — the family, and then a new size: the grid a terminal is drawn on is
    // as wide as the face it uses, so a different face is a different grid and the number of columns
    // that fit changes with it.
    void nextTick(() => {
      if (terminal) {
        terminal.options.fontFamily = preferences.wornTerminalFont;
        applySize();
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
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  window.removeEventListener("pointerup", onWindowPointerUp);
  observer?.disconnect();
  unregister?.();
  osc7?.dispose();
  terminal?.dispose();
});
</script>

<template>
  <!--
    The surface, and the two things that stand over it: the find bar, and the toolbar about a
    selection. The wrapper exists for them — both are positioned against the terminal rather than
    pushing it, so neither a search nor a selection re-fits the terminal and reflows the very output
    being worked with.
  -->
  <div ref="paneBox" class="relative min-h-0 min-w-0 flex-1">
    <div
      ref="container"
      class="terminal-surface h-full overflow-hidden bg-[var(--terminal-background)]"
      :class="paletteClass"
      @contextmenu.capture="onContextMenu"
      @pointerdown="onSurfacePointerDown"
    ></div>

    <!--
      The toolbar over a selection: the same three things the right-click menu offers — copy, hand
      the selection to the shell, find — one gesture closer. Icons only, so each button says what it
      is through its label and its tooltip rather than through a word the rows behind it would show
      through.
    -->
    <div
      v-if="toolsAt"
      ref="toolsBox"
      role="toolbar"
      :aria-label="t('terminal.selection.label')"
      class="absolute z-20 flex items-center gap-0.5 rounded-md border border-border bg-card/95 p-0.5 shadow-lg"
      :style="{ left: `${toolsAt.left}px`, top: `${toolsAt.top}px` }"
    >
      <button
        v-for="entry in toolEntries"
        :key="entry.id"
        type="button"
        class="grid size-6 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
        :title="toolsTooltip(entry)"
        :aria-label="t(entry.labelKey)"
        :disabled="entry.disabled === true"
        @click="runTool(entry)"
      >
        <component :is="entry.icon" v-if="entry.icon" :size="13" />
      </button>
    </div>

    <div
      v-if="searching"
      class="absolute top-2 right-3 z-20 flex items-center gap-1 rounded-md border border-border bg-card/95 px-1.5 py-1 shadow-lg"
    >
      <input
        ref="searchInput"
        v-model="query"
        spellcheck="false"
        class="w-56 rounded-sm border border-border bg-background px-1.5 py-0.5 text-[11px] focus:border-primary focus:outline-none"
        :placeholder="t('terminal.search.placeholder')"
        :aria-label="t('terminal.search.label')"
        @input="runSearch()"
        @keydown.enter.prevent="runSearch($event.shiftKey)"
        @keydown.esc.prevent="closeSearch()"
      />

      <span class="min-w-10 shrink-0 text-center font-mono text-[10px] text-muted-foreground">
        {{
          results === null
            ? ""
            : results.count === 0
              ? t("terminal.search.none")
              : `${results.index + 1}/${results.count}`
        }}
      </span>

      <button
        type="button"
        class="grid size-5 place-items-center rounded-sm transition-colors"
        :class="caseSensitive ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent'"
        :aria-pressed="caseSensitive"
        :title="t('terminal.search.caseSensitive')"
        :aria-label="t('terminal.search.caseSensitive')"
        @click="
          caseSensitive = !caseSensitive;
          runSearch();
        "
      >
        Aa
      </button>
      <button
        type="button"
        class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('terminal.search.previous')"
        :aria-label="t('terminal.search.previous')"
        @click="runSearch(true)"
      >
        <ChevronUp :size="11" />
      </button>
      <button
        type="button"
        class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('terminal.search.next')"
        :aria-label="t('terminal.search.next')"
        @click="runSearch(false)"
      >
        <ChevronDown :size="11" />
      </button>
      <button
        type="button"
        class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('terminal.search.close')"
        :aria-label="t('terminal.search.close')"
        @click="closeSearch()"
      >
        <X :size="11" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/*
 * Breathing room around the text.
 *
 * Two things are load-bearing here.
 *
 * 1. **The padding goes on the terminal element, not on the container.** FitAddon subtracts the
 *    padding of the element it drew into, so this keeps the column count right; padding on the
 *    container would be counted as columns that do not exist.
 * 2. **Every layer of the terminal is given the palette's background.** `.xterm-viewport` is
 *    absolutely positioned and therefore covers the padding box — absolute positioning ignores an
 *    ancestor's padding — and its own default background is black. Without this the padding shows
 *    up as a black frame around a light terminal.
 */
.terminal-surface :deep(.xterm) {
  padding: 10px 12px;
}

.terminal-surface :deep(.xterm),
.terminal-surface :deep(.xterm-viewport),
.terminal-surface :deep(.xterm-screen) {
  background-color: var(--terminal-background);
}
</style>
