/**
 * Completion at the terminal's own prompt.
 *
 * The terminal draws what the shell echoes; this follows what was typed and offers the same kinds of
 * candidates the command bar does, anchored at the cursor instead of at a bar. Two rules shape it:
 *
 * * **A menu being visible is not the same as owning the keyboard.** Enter always runs the line.
 *   Tab takes a static candidate only while the menu is open; with the menu closed it goes to the
 *   shell. Arrow keys navigate the menu only while it is open. Escape closes it when it is open.
 * * **The line is only completed when it is known.** A history recall, a tab completion, a paste or
 *   an escape sequence changes the line without telling this side what it became; from then until
 *   Enter, Ctrl+U or Ctrl+C there is no completion at all.
 *
 * Accepting a candidate rewrites the shell's own line the way readline does: Ctrl+U (kill to the
 * beginning of the line) followed by the new line. That is why the gate above matters — the rewrite
 * assumes the line the shell has is the line this side tracked.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch, type ComputedRef, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import type { Terminal } from "@xterm/xterm";

import { useRemotePaths } from "@/components/commands/useRemotePaths";
import { acceptanceBoost, recordAccepted } from "@/lib/commands/acceptance";
import { COMMAND_CATALOG, descriptionKeyFor } from "@/lib/commands/catalog";
import { buildCommandCompletions, type CompletionCandidate } from "@/lib/commands/completion";
import { loadSpecs, specsLoaded } from "@/lib/commands/specs/load";
import { subcommandKeyFor, subcommandsOf } from "@/lib/commands/subcommands";
import { noteFor, wordsFor } from "@/lib/commands/words";
import { matchesShortcut } from "@/lib/shortcuts/binding";
import { bindingFor } from "@/lib/shortcuts/registry";
import { InputLine } from "@/lib/terminal/inputLine";
import { interactivePromptNearCursor } from "@/lib/terminal/promptGuard";
import { useAppStore } from "@/stores/app";
import { useCommandsStore } from "@/stores/commands";
import { useCompletionStore } from "@/stores/completion";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore, type TerminalTab } from "@/stores/terminals";
import { useWordsStore } from "@/stores/words";

/** How long after the last keystroke the line is read, so a fast word is one request. */
const DEBOUNCE_MS = 90;
/** How quiet the terminal has to be before a list is drawn over it. */
const IDLE_MS = 120;
/** The smallest useful menu, in pixels; below this there is no room for a list. */
const MIN_MENU_PX = 200;

export interface MenuAnchor {
  readonly left: number;
  readonly top?: number;
  readonly bottom?: number;
  readonly maxHeight?: number;
}

export interface GhostAnchor {
  readonly left: number;
  readonly top: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
}

export interface TerminalCompletionOptions {
  readonly handle: () => string;
  readonly active: () => boolean;
  readonly interactive: () => boolean;
  readonly focused: () => boolean;
  readonly terminal: () => Terminal | undefined;
  readonly pane: () => HTMLElement | null;
  readonly send: (data: string) => void;
}

export interface TerminalCompletion {
  readonly line: Ref<string>;
  readonly candidates: Ref<readonly CompletionCandidate[]>;
  readonly active: Ref<number>;
  readonly visible: ComputedRef<boolean>;
  readonly anchor: Ref<MenuAnchor | null>;
  readonly ghost: Ref<string | null>;
  readonly ghostAnchor: Ref<GhostAnchor | null>;
  feed(data: string): void;
  output(): void;
  onKey(event: KeyboardEvent): boolean;
  /** Takes the row at `index` into the line, or does nothing when the row is gone. */
  accept(index: number): void;
  reposition(): void;
  hide(): void;
  reset(): void;
}

export function useTerminalCompletion(options: TerminalCompletionOptions): TerminalCompletion {
  const { t, te } = useI18n();
  const app = useAppStore();
  const commands = useCommandsStore();
  const completion = useCompletionStore();
  const files = useFilesStore();
  const preferences = usePreferencesStore();
  const terminals = useTerminalsStore();
  const words = useWordsStore();

  const input = new InputLine();
  const line = ref("");
  const caret = ref(0);
  const trusted = ref(true);
  const candidates = ref<readonly CompletionCandidate[]>([]);
  const active = ref(0);
  const chosen = ref(false);
  const anchor = ref<MenuAnchor | null>(null);
  const ghost = ref<string | null>(null);
  const ghostAnchor = ref<GhostAnchor | null>(null);

  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let lastOutputAt = 0;
  let refusedLine: string | null = null;

  const visible = computed(() => candidates.value.length > 0 && anchor.value !== null);

  const remote = useRemotePaths(() => {
    const tab = terminals.tabs.find(
      (entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === options.handle(),
    );
    const panel = tab?.panel?.handle ?? null;
    const browser = panel === null ? null : files.browser(panel);
    return browser === null ? null : { handle: browser.handle, cwd: browser.shellPath };
  });

  function build(text: string): readonly CompletionCandidate[] {
    return buildCommandCompletions({
      line: text,
      caret: text.length,
      sources: {
        saved: commands.groups.flatMap((group) => group.commands),
        history: commands.history,
        catalog: COMMAND_CATALOG,
      },
      describe: (name) => (te(descriptionKeyFor(name)) ? t(descriptionKeyFor(name)) : null),
      subcommands: (command) => wordsFor(command, words.words, subcommandsOf(command)),
      // The same words without the shipped table: a specification does not replace what somebody
      // added for a command it happens to know.
      userWords: (command) => wordsFor(command, words.words, []),
      describeSubcommand: (command, word) => {
        const own = noteFor(command, word, words.words);
        if (own !== null) {
          return own;
        }
        const key = subcommandKeyFor(command, word);
        return te(key) ? t(key) : null;
      },
      kindLabel: (kind) => t(`commands.completion.kind.${kind}`),
      acceptance: acceptanceBoost,
      paths: {
        cwd: remote.base,
        entries: (directory) => {
          const asked = `${text.length}@${text}`;
          remote.warm(directory, () => {
            if (`${caret.value}@${line.value}` === asked) {
              schedule(0);
            }
          });
          return remote.entries(directory);
        },
      },
      specs: specsLoaded(),
      sessionHandle: options.handle(),
      cwd: terminals.shellDirectory(options.handle()) ?? remote.base,
      values: {
        cached: (key) => completion.cached(key),
        warm: (slot) => completion.warm(slot),
      },
      // A word and the host's directory listing can both fill the same argument; the list is long
      // enough to hold both without one silently pushing the other out of the menu.
      limit: 120,
    });
  }

  function enabled(): boolean {
    const terminal = options.terminal();
    return (
      preferences.commandCompletion &&
      options.active() &&
      options.interactive() &&
      options.focused() &&
      terminal !== undefined &&
      terminal.buffer.active.type === "normal"
    );
  }

  /**
   * Whether the cursor is sitting on a line that is asking for an answer rather than for a command.
   * Completion there would offer to rewrite a credential or a menu choice.
   */
  function promptOnScreen(): boolean {
    const terminal = options.terminal();
    if (terminal === undefined) {
      return false;
    }
    const buffer = terminal.buffer.active;
    const row = buffer.baseY + buffer.cursorY;
    return interactivePromptNearCursor(row, (at) => buffer.getLine(at)?.translateToString(true) ?? "");
  }

  function feed(data: string): void {
    const state = input.feed(data);
    line.value = state.line;
    caret.value = state.line.length;
    trusted.value = state.trusted;

    if (state.executed) {
      hide();
      return;
    }
    if (refusedLine !== null && refusedLine !== state.line) {
      refusedLine = null;
    }
    schedule(DEBOUNCE_MS);
  }

  function output(): void {
    lastOutputAt = Date.now();
    ghost.value = null;
    ghostAnchor.value = null;
  }

  function schedule(delay: number): void {
    if (!enabled()) {
      hide();
      return;
    }
    if (refreshTimer !== undefined) {
      clearTimeout(refreshTimer);
    }
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined;
      refresh(0);
    }, delay);
  }

  function refresh(attempt: number): void {
    if (!enabled()) {
      hide();
      return;
    }
    if (line.value === "" || !trusted.value) {
      hide();
      return;
    }
    // A list drawn while a command is still printing is a list over somebody's output. Wait for the
    // terminal to be quiet; the next attempt is a short one rather than a dropped keystroke.
    if (Date.now() - lastOutputAt < IDLE_MS && attempt < 6) {
      schedule(IDLE_MS);
      return;
    }
    if (refusedLine !== null && refusedLine === line.value) {
      hide();
      return;
    }
    if (promptOnScreen()) {
      hide();
      return;
    }

    const found = build(line.value);
    if (found.length === 0) {
      hide();
      updateGhost();
      return;
    }

    candidates.value = found;
    active.value = Math.min(active.value, found.length - 1);
    chosen.value = false;
    anchor.value = readMenuAnchor();
    ghost.value = null;
    ghostAnchor.value = null;
    if (anchor.value === null) {
      hide();
    }
  }

  function updateGhost(): void {
    const text = line.value;
    if (text === "" || !trusted.value || /\s$/.test(text) || !enabled()) {
      ghost.value = null;
      ghostAnchor.value = null;
      return;
    }
    if (Date.now() - lastOutputAt < IDLE_MS) {
      ghost.value = null;
      ghostAnchor.value = null;
      return;
    }
    if (promptOnScreen()) {
      ghost.value = null;
      ghostAnchor.value = null;
      return;
    }

    const extension = prefixExtension(text);
    ghost.value = extension === null ? null : extension.slice(text.length);
    ghostAnchor.value = ghost.value === null ? null : readGhostAnchor();
  }

  /** A whole line somebody ran or saved that starts with exactly what is on the line. */
  function prefixExtension(text: string): string | null {
    for (const entry of commands.history) {
      if (entry.command.length > text.length && entry.command.startsWith(text)) {
        return entry.command;
      }
    }
    for (const group of commands.groups) {
      for (const snippet of group.commands) {
        if (!snippet.enabled) {
          continue;
        }
        if (snippet.command.length > text.length && snippet.command.startsWith(text)) {
          return snippet.command;
        }
      }
    }
    return null;
  }

  function onKey(event: KeyboardEvent): boolean {
    if (event.type !== "keydown" || !options.active() || !options.interactive()) {
      return false;
    }

    if (visible.value) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        move(1);
        return true;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        move(-1);
        return true;
      }
      if (event.key === "Tab") {
        event.preventDefault();
        accept(active.value);
        return true;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        dismiss();
        return true;
      }
      if (event.key === "Enter") {
        // Enter is the shell's: the list gets out of its way and the line runs.
        hide();
        return false;
      }
      if (["ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"].includes(event.key)) {
        hide();
      }
      return false;
    }

    if (matchesShortcut(event, bindingFor("commands.completionTrigger", preferences.keybindings), app.platform)) {
      event.preventDefault();
      open();
      return true;
    }
    if (
      ghost.value !== null &&
      event.key === "ArrowRight" &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.metaKey
    ) {
      event.preventDefault();
      acceptGhost();
      return true;
    }
    if (event.key === "Enter" || event.key === "Escape" || event.key === "Tab") {
      ghost.value = null;
      ghostAnchor.value = null;
    }
    return false;
  }

  function open(): void {
    refusedLine = null;
    if (refreshTimer !== undefined) {
      clearTimeout(refreshTimer);
      refreshTimer = undefined;
    }
    refresh(0);
  }

  function dismiss(): void {
    refusedLine = line.value;
    hide();
  }

  function move(delta: number): void {
    const count = candidates.value.length;
    if (count === 0) {
      return;
    }
    const from = chosen.value ? active.value : delta > 0 ? -1 : 0;
    active.value = (from + delta + count) % count;
    chosen.value = true;
  }

  function accept(index: number): void {
    const candidate = candidates.value[index];
    if (candidate === undefined) {
      return;
    }
    const rest = line.value.slice(caret.value);
    const tail = candidate.trailingSpace && rest === "" ? " " : "";
    const next = `${line.value.slice(0, candidate.from)}${candidate.insert}${tail}${rest}`;

    recordAccepted(candidate.insert);
    input.setLine(next);
    line.value = next;
    caret.value = next.length;
    hide();
    options.send(`\x15${next}`);
    schedule(0);
  }

  function acceptGhost(): void {
    const text = ghost.value;
    if (text === null) {
      return;
    }
    const next = line.value + text;
    input.setLine(next);
    line.value = next;
    caret.value = next.length;
    ghost.value = null;
    ghostAnchor.value = null;
    options.send(text);
  }

  function hide(): void {
    candidates.value = [];
    active.value = 0;
    chosen.value = false;
    anchor.value = null;
    ghost.value = null;
    ghostAnchor.value = null;
  }

  function reset(): void {
    input.reset();
    line.value = "";
    caret.value = 0;
    trusted.value = true;
    refusedLine = null;
    lastOutputAt = 0;
    hide();
  }

  function reposition(): void {
    if (visible.value) {
      anchor.value = readMenuAnchor();
    }
    if (ghost.value !== null) {
      ghostAnchor.value = readGhostAnchor();
    }
  }

  function cursorFrame(): {
    left: number;
    top: number;
    cellWidth: number;
    cellHeight: number;
    paneWidth: number;
    paneHeight: number;
  } | null {
    const terminal = options.terminal();
    const pane = options.pane();
    if (terminal === undefined || pane === null) {
      return null;
    }
    const screen = terminal.element?.querySelector(".xterm-screen");
    if (screen === undefined || screen === null) {
      return null;
    }
    const paneRect = pane.getBoundingClientRect();
    const screenRect = screen.getBoundingClientRect();
    const cellWidth = screenRect.width / terminal.cols;
    const cellHeight = screenRect.height / terminal.rows;
    if (!(cellWidth > 0) || !(cellHeight > 0)) {
      return null;
    }
    const buffer = terminal.buffer.active;
    return {
      left: screenRect.left - paneRect.left + buffer.cursorX * cellWidth,
      top: screenRect.top - paneRect.top + buffer.cursorY * cellHeight,
      cellWidth,
      cellHeight,
      paneWidth: pane.clientWidth,
      paneHeight: pane.clientHeight,
    };
  }

  function readMenuAnchor(): MenuAnchor | null {
    const frame = cursorFrame();
    if (frame === null) {
      return null;
    }
    const below = frame.paneHeight - (frame.top + frame.cellHeight);
    const placeBelow = below >= MIN_MENU_PX || below >= frame.top;
    const left = Math.max(4, Math.min(frame.left, Math.max(4, frame.paneWidth - 300)));
    if (placeBelow) {
      return {
        left,
        top: frame.top + frame.cellHeight + 4,
        maxHeight: Math.max(120, below - 8),
      };
    }
    return {
      left,
      bottom: frame.paneHeight - frame.top + 4,
      maxHeight: Math.max(120, frame.top - 8),
    };
  }

  function readGhostAnchor(): GhostAnchor | null {
    const frame = cursorFrame();
    if (frame === null) {
      return null;
    }
    return {
      left: frame.left + frame.cellWidth,
      top: frame.top,
      cellWidth: frame.cellWidth,
      cellHeight: frame.cellHeight,
    };
  }

  onMounted(() => {
    if (!commands.loaded) {
      void commands.load();
    }
    if (!words.loaded) {
      void words.load();
    }
    void loadSpecs().then(() => {
      if (line.value !== "" && enabled()) {
        schedule(0);
      }
    });
  });

  watch(
    () => completion.version,
    () => {
      if (line.value !== "" && enabled()) {
        schedule(0);
      }
    },
  );

  watch(
    () => commands.loaded || words.loaded,
    () => {
      if (line.value !== "" && enabled()) {
        schedule(0);
      }
    },
  );

  watch(
    () => preferences.commandCompletion,
    (on) => {
      if (!on) {
        hide();
      }
    },
  );

  onBeforeUnmount(() => {
    if (refreshTimer !== undefined) {
      clearTimeout(refreshTimer);
      refreshTimer = undefined;
    }
  });

  return {
    line,
    candidates,
    active,
    visible,
    anchor,
    ghost,
    ghostAnchor,
    feed,
    output,
    onKey,
    accept,
    reposition,
    hide,
    reset,
  };
}
