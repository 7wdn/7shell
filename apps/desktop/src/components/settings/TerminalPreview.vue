<script setup lang="ts">
/**
 * A terminal, as the settings beside it will make it.
 *
 * Not a second terminal engine: a terminal drawn as a terminal is, out of the same tokens the real
 * panes read — the background, the text, the cursor and the sixteen colours are the stylesheet's, and
 * the face and the size are the settings the page around this is about. What it shows is what the
 * settings do, which is the whole reason a page of sizes and families has a picture at the top of it.
 *
 * It is not a terminal and does not pretend to be one: no shell, no input, nothing scrolling. The
 * lines are a session somebody might have — a prompt, a command, its output, and one line in each of
 * the colours a terminal actually uses — chosen so that a font that cannot tell `l` from `1` and a
 * palette whose red is unreadable on its own background both show up here rather than in a shell.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { findTerminalScheme, schemeCssVars } from "@/lib/terminal/schemes";
import { usePreferencesStore } from "@/stores/preferences";

const { t } = useI18n();
const preferences = usePreferencesStore();

/**
 * The class that says which half of the palette to read.
 *
 * The same choice the real panes make, and the same class: a dark terminal in a light window is a
 * decision somebody is allowed to make, so the preview follows the terminal's own setting rather than
 * the window's — and the stylesheet already turns that class into `--terminal-background` and the rest.
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
  if (preferences.terminalSchemeSource !== "custom") {
    return {};
  }
  const id =
    preferences.resolvedTerminalTheme === "dark" ? preferences.terminalSchemeDark : preferences.terminalSchemeLight;
  const scheme = findTerminalScheme(id, preferences.terminalCustomSchemes);
  if (!scheme) {
    return {};
  }
  const vars = schemeCssVars(scheme);
  if (preferences.terminalSchemeBackground === "scheme") {
    vars["--terminal-background"] = scheme.background;
    vars["--terminal-foreground"] = scheme.foreground;
  }
  return vars;
});

const lines = computed(() => [
  { text: t("settings.terminalPreview.lines.prompt"), colour: "var(--terminal-bright-green)", bold: true },
  { text: t("settings.terminalPreview.lines.command"), colour: "var(--terminal-foreground)", bold: false },
  { text: t("settings.terminalPreview.lines.output"), colour: "var(--terminal-cyan)", bold: false },
  { text: t("settings.terminalPreview.lines.warning"), colour: "var(--terminal-yellow)", bold: false },
  { text: t("settings.terminalPreview.lines.error"), colour: "var(--terminal-red)", bold: false },
  { text: t("settings.terminalPreview.lines.plain"), colour: "var(--terminal-foreground)", bold: false },
]);

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
</script>

<template>
  <div
    data-terminal-preview
    class="min-w-0 overflow-hidden rounded-md border border-border"
    :class="half"
    :style="{ background: 'var(--terminal-background)', ...schemeVars, ...boxStyle }"
  >
    <!--
      One line per colour, in the monospace face and at the size the settings hold. Not selectable: a
      picture of a terminal is a picture, and a drag across a page of settings is a drag towards a
      control rather than towards this.
    -->
    <pre class="m-0 overflow-hidden font-mono whitespace-pre select-none" :style="style"><span
        v-for="line in lines"
        :key="line.text"
        :style="{ color: line.colour, fontWeight: line.bold ? preferences.terminalBoldFontWeight : undefined }"
        >{{ line.text }}
</span
      ></pre>
  </div>
</template>
