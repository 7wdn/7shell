<script setup lang="ts">
/**
 * A terminal, as the settings beside it will make it.
 *
 * Not a second terminal engine: a terminal drawn as a terminal is, out of the same tokens the real
 * panes read. What it shows is what the settings do — the face, the size and the spacing, the
 * sixteen colours a scheme brings, the cursor in the shape and blink the settings name (with an
 * unfocused one on the last line, because the inactive style is its own question), bold text in
 * the bright colours or not, every line lifted to the minimum contrast the setting demands, and a
 * word drawn the way the highlight rules would draw it. Which is the whole reason a page of sizes
 * and families has a picture at the top of it.
 *
 * It is not a terminal and does not pretend to be one: no shell, no input, nothing scrolling. The
 * lines are a session somebody might have — a prompt, a command, its output, and one line in each
 * of the colours a terminal actually uses — chosen so that a font that cannot tell `l` from `1`
 * and a palette whose red is unreadable on its own background both show up here rather than in a
 * shell.
 */
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { ensureMinimumContrast } from "@/lib/terminal/contrast";
import { compileHighlightRules, highlightMatches } from "@/lib/terminal/highlight";
import { findTerminalScheme, schemeCssVars, withAlpha } from "@/lib/terminal/schemes";
import { usePreferencesStore } from "@/stores/preferences";
import { backgroundIsSet } from "@/types/app";

const { t } = useI18n();
const preferences = usePreferencesStore();

/**
 * The class that says which half of the palette to read.
 *
 * The same choice the real panes make, and the same class: a dark terminal in a light window is a
 * decision somebody is allowed to make, so the preview follows the terminal's own setting rather
 * than the window's — and the stylesheet already turns that class into `--terminal-background` and the rest.
 */
const half = computed(() => (preferences.resolvedTerminalTheme === "dark" ? "terminal-dark" : "terminal-light"));

/**
 * The sixteen colours a scheme brings, written over the ones the palette class declares.
 *
 * A scheme that only supplies colours leaves the background and text to the palette, exactly as the
 * pane does — a preview that showed the whole scheme in that mode would be promising a look the
 * terminal will not draw.
 */
const schemeVars = computed<Record<string, string>>(() => {
  const vars: Record<string, string> = {};
  if (preferences.terminalSchemeSource === "custom") {
    const id =
      preferences.resolvedTerminalTheme === "dark" ? preferences.terminalSchemeDark : preferences.terminalSchemeLight;
    const scheme = findTerminalScheme(id, preferences.terminalCustomSchemes);
    if (scheme) {
      Object.assign(vars, schemeCssVars(scheme));
      if (preferences.terminalSchemeBackground === "scheme") {
        vars["--terminal-background"] = scheme.background;
        vars["--terminal-foreground"] = scheme.foreground;
      }
    }
  }

  // The text colour chosen beside the source cards, shown exactly where the pane would use it: a
  // scheme that replaced the whole look brought its own and keeps it.
  const chosen =
    preferences.resolvedTerminalTheme === "dark"
      ? preferences.terminalForegroundDark
      : preferences.terminalForegroundLight;
  if (chosen && vars["--terminal-foreground"] === undefined) {
    vars["--terminal-foreground"] = chosen;
  }

  return vars;
});

/**
 * What the palette's colours are, as values the contrast arithmetic can read.
 *
 * The container carries the resolved answer — a scheme's own hex on its style, the stylesheet's
 * tokens otherwise — and the computed style reports those declarations as written: hex from a
 * scheme, `oklch()` from the palette. Both parse, so the lines can be drawn at the contrast the
 * setting asks for rather than raw. When the read comes back empty (a document without the
 * stylesheet) the lines fall back to the variables themselves and the contrast step is skipped —
 * a missing answer is not an answer to invent.
 */
const box = ref<HTMLElement | null>(null);
const probed = ref<Record<string, string>>({});

const PROBE_KEYS = [
  "--terminal-background",
  "--terminal-foreground",
  "--terminal-red",
  "--terminal-yellow",
  "--terminal-green",
  "--terminal-bright-green",
  "--terminal-cyan",
] as const;

watch(
  [half, schemeVars],
  async () => {
    await nextTick();
    const element = box.value;
    if (!element) {
      return;
    }
    const style = getComputedStyle(element);
    const next: Record<string, string> = {};
    for (const key of PROBE_KEYS) {
      next[key] = style.getPropertyValue(key).trim();
    }
    probed.value = next;
  },
  { immediate: true },
);

type ProbeKey = (typeof PROBE_KEYS)[number];

function paletteColour(key: ProbeKey): string | null {
  const value = probed.value[key];
  return value ? value : null;
}

type LineKind = "prompt" | "command" | "output" | "warning" | "error" | "plain";

const LINE_COLOUR_KEYS: Record<Exclude<LineKind, "prompt">, ProbeKey> = {
  command: "--terminal-foreground",
  output: "--terminal-cyan",
  warning: "--terminal-yellow",
  error: "--terminal-red",
  plain: "--terminal-foreground",
};

/** One line's colour: the key the line means, lifted to the minimum contrast the setting demands. */
function lineColour(kind: LineKind): string {
  const key =
    kind === "prompt" && !preferences.terminalBoldBright
      ? "--terminal-green"
      : kind === "prompt"
        ? "--terminal-bright-green"
        : LINE_COLOUR_KEYS[kind];
  const colour = paletteColour(key);
  if (!colour) {
    return `var(${key})`;
  }
  return ensureMinimumContrast(
    colour,
    paletteColour("--terminal-background") ?? "#000000",
    preferences.terminalMinContrast,
  );
}

const lines = computed<{ kind: LineKind; text: string; bold: boolean }[]>(() => [
  { kind: "prompt", text: t("settings.terminalPreview.lines.prompt"), bold: true },
  { kind: "command", text: t("settings.terminalPreview.lines.command"), bold: false },
  { kind: "output", text: t("settings.terminalPreview.lines.output"), bold: false },
  { kind: "warning", text: t("settings.terminalPreview.lines.warning"), bold: false },
  { kind: "error", text: t("settings.terminalPreview.lines.error"), bold: false },
  { kind: "plain", text: t("settings.terminalPreview.lines.plain"), bold: false },
]);

/**
 * The words the highlight rules name, as the segments the line splits into.
 *
 * The same reader the panes use, over the same rules, so a preview cannot disagree with a terminal
 * about what a rule catches. The colour a rule brings is drawn the way the mode says: a wash under
 * the word, or the word repainted.
 */
interface RowPart {
  readonly text: string;
  readonly highlight?: string;
}

const compiledRules = computed(() => compileHighlightRules(preferences.terminalHighlightRules));

function highlightSegments(text: string): readonly RowPart[] {
  const matches = [...highlightMatches(text, compiledRules.value)].sort((first, second) => first.start - second.start);
  if (matches.length === 0) {
    return [{ text }];
  }
  const parts: RowPart[] = [];
  let at = 0;
  for (const match of matches) {
    if (match.start > at) {
      parts.push({ text: text.slice(at, match.start) });
    }
    parts.push({ text: text.slice(match.start, match.end), highlight: match.color });
    at = match.end;
  }
  if (at < text.length) {
    parts.push({ text: text.slice(at) });
  }
  return parts;
}

function partStyle(part: RowPart, base: string): Record<string, string> {
  if (!part.highlight) {
    return { color: base };
  }
  return preferences.terminalHighlightMode === "background"
    ? { color: base, background: withAlpha(part.highlight, 0.35) }
    : { color: part.highlight };
}

const rows = computed(() =>
  lines.value.map((line, index) => ({
    ...line,
    colour: lineColour(line.kind),
    parts: highlightSegments(line.text),
    activeCursor: index === 0,
    inactiveCursor: index === lines.value.length - 1 && preferences.terminalCursorInactiveStyle !== "none",
  })),
);

/**
 * The cursor the settings describe, drawn as the renderer draws it: a filled cell, a thin bar, a
 * rule under the cell, or — only ever on the unfocused line — a hollow outline of the cell.
 */
function cursorStyle(active: boolean): Record<string, string> {
  const colour = paletteColour("--terminal-foreground") ?? "var(--terminal-foreground)";
  const shape = active ? preferences.terminalCursorStyle : preferences.terminalCursorInactiveStyle;
  const seat = { display: "inline-block", verticalAlign: "-0.15em" };
  switch (shape) {
    case "bar":
      return { ...seat, width: "2px", height: "1em", background: colour };
    case "underline":
      return { ...seat, width: "1ch", height: "2px", background: colour };
    case "outline":
      return { ...seat, width: "1ch", height: "1em", border: `1px solid ${colour}` };
    default:
      return { ...seat, width: "1ch", height: "1em", background: colour };
  }
}

/**
 * The face, the size and the spacing the settings say.
 *
 * The line height is the terminal's own rather than a number chosen for the picture: a preview that
 * stayed comfortable at 1.45 would hide the one thing somebody moving that slider is looking for.
 * The bold line uses the bold weight, which is the other setting this block can show.
 */
const style = computed(() => ({
  fontFamily: preferences.wornTerminalFont,
  fontSize: `${preferences.terminalFontSize}px`,
  fontWeight: preferences.terminalFontWeight,
  letterSpacing: `${preferences.terminalLetterSpacing}px`,
  lineHeight: preferences.terminalLineHeight,
}));

const boxStyle = computed(() => ({
  padding: `${preferences.terminalPaddingY}px ${preferences.terminalPaddingX}px`,
}));

/**
 * What the picture's own layer behind everything does to this surface.
 *
 * The same answer the real pane gives: with a picture behind the window, the pane's colour keeps
 * only what the transparency slider leaves of it. Drawn with the same CSS the pane uses, so the
 * two cannot disagree about how much shows through.
 */
const background = computed(() => {
  if (!backgroundIsSet(preferences.backgroundImage)) {
    return "var(--terminal-background)";
  }
  const transparency = Math.min(Math.max(Math.round(preferences.backgroundImage.transparency), 0), 100);
  return `color-mix(in oklab, var(--terminal-background) ${100 - transparency}%, transparent)`;
});
</script>

<template>
  <div
    ref="box"
    data-terminal-preview
    class="min-w-0 overflow-hidden rounded-md border border-border"
    :class="half"
    :style="{ background, ...schemeVars, ...boxStyle }"
  >
    <!--
      One line per colour, in the monospace face and at the size the settings hold. Not selectable: a
      picture of a terminal is a picture, and a drag across a page of settings is a drag towards a
      control rather than towards this.
    -->
    <pre class="m-0 overflow-hidden font-mono whitespace-pre select-none" :style="style"><span
        v-for="row in rows"
        :key="row.kind"
        :style="{ fontWeight: row.bold ? preferences.terminalBoldFontWeight : undefined }"
        ><span
          v-for="(part, index) in row.parts"
          :key="index"
          :style="partStyle(part, row.colour)"
          >{{ part.text }}</span
        ><span
          v-if="row.activeCursor"
          class="preview-cursor"
          :class="{ blink: preferences.terminalCursorBlink }"
          :style="cursorStyle(true)"
        ></span><span
          v-else-if="row.inactiveCursor"
          class="preview-cursor"
          :style="cursorStyle(false)"
        ></span><span
          >{{ "\n" }}</span
        ></span
      ></pre>
  </div>
</template>

<style scoped>
/* The cadence xterm keeps for a blinking cursor, one step rather than a fade. */
.preview-cursor {
  display: inline-block;
}

.preview-cursor.blink {
  animation: preview-cursor-blink 1.1s steps(1) infinite;
}

@keyframes preview-cursor-blink {
  50% {
    opacity: 0;
  }
}
</style>
