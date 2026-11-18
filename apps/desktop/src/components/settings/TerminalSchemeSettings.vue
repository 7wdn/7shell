<script setup lang="ts">
/**
 * The terminal's colour source, its two scheme slots, what a scheme replaces, and the schemes
 * somebody brought with them.
 *
 * One block rather than five rows because the answers depend on each other: the schemes only matter
 * once the source says "custom", the background choice only matters once a scheme is chosen, and the
 * imported list is the catalogue those choices can name. Drawn this way, controls that do nothing
 * are not on screen pretending to be choices.
 *
 * The catalogue is the build's; imported schemes are stored in the settings file beside the ids
 * that name them, and an id that resolves to nothing falls back to the interface palette.
 */
import { Plus, RotateCcw, Trash2 } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import TerminalSchemeImportDialog from "@/components/settings/TerminalSchemeImportDialog.vue";
import TerminalSchemePicker from "@/components/settings/TerminalSchemePicker.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { type ImportedTerminalScheme, mergeImportedSchemes } from "@/lib/terminal/importScheme";
import { TERMINAL_SCHEME_PRESETS } from "@/lib/terminal/presets";
import { TERMINAL_COLOR_SCHEMES, type TerminalColorScheme } from "@/lib/terminal/schemes";
import { usePreferencesStore } from "@/stores/preferences";
import {
  TERMINAL_CUSTOM_SCHEME_LIMIT,
  TERMINAL_SCHEME_PROFILE_LIMIT,
  type TerminalCustomScheme,
  type TerminalSchemeBackground,
  type TerminalSchemeProfile,
  type TerminalSchemeSource,
} from "@/types/app";

const { t } = useI18n();
const preferences = usePreferencesStore();

const sources: readonly { value: TerminalSchemeSource; labelKey: string; hintKey: string }[] = [
  { value: "host", labelKey: "settings.schemes.sourceHost", hintKey: "settings.schemes.sourceHostHint" },
  { value: "custom", labelKey: "settings.schemes.sourceCustom", hintKey: "settings.schemes.sourceCustomHint" },
];

const backgrounds: readonly { value: TerminalSchemeBackground; labelKey: string }[] = [
  { value: "scheme", labelKey: "settings.schemes.backgroundScheme" },
  { value: "host", labelKey: "settings.schemes.backgroundHost" },
];

/** Everything the pickers can offer: the shipped catalogue, then what was imported. */
const schemes = computed<readonly TerminalColorScheme[]>(() => [
  ...TERMINAL_COLOR_SCHEMES,
  ...preferences.terminalCustomSchemes,
]);

const imported = computed<readonly TerminalCustomScheme[]>(() => preferences.terminalCustomSchemes);

const importing = ref(false);
const removing = ref<TerminalCustomScheme | null>(null);
const profileName = ref("");
const removingProfile = ref<TerminalSchemeProfile | null>(null);

/** The saved answers and the ready-made ones, in the order the list draws them. */
const profiles = computed(() => [
  ...TERMINAL_SCHEME_PRESETS.map((preset) => ({
    id: preset.id,
    name: preset.nameKey ? t(preset.nameKey) : preset.name,
    profile: preset.profile,
    builtin: true,
  })),
  ...preferences.terminalSchemeProfiles.map((profile) => ({
    id: profile.id,
    name: profile.name,
    profile,
    builtin: false,
  })),
]);

/** The colours a profile wears on its dark half, for the strip beside its name. */
function profileSwatches(profile: TerminalSchemeProfile): readonly string[] {
  const scheme = schemes.value.find((entry) => entry.id === profile.schemeDark);
  return scheme ? scheme.colors.slice(0, 16) : [];
}

function saveProfile(): void {
  const taken = profiles.value.map((entry) => entry.id);
  if (preferences.saveTerminalSchemeProfile(profileName.value, taken)) {
    profileName.value = "";
  }
}

function removeProfile(): void {
  if (removingProfile.value) {
    preferences.removeTerminalSchemeProfile(removingProfile.value.id);
  }
  removingProfile.value = null;
}

/**
 * Takes what the dialog parsed and gives each scheme an id the catalogue does not already use.
 *
 * The ids are computed against the current list and against each other, because two schemes in one
 * paste may share a name; the store's own deduplication is the second half of the same rule, for a
 * settings file that was edited by hand.
 */
function takeImported(parsed: readonly ImportedTerminalScheme[]): void {
  const taken = new Set(schemes.value.map((scheme) => scheme.id));
  const next: TerminalCustomScheme[] = mergeImportedSchemes(
    parsed,
    taken,
    preferences.terminalCustomSchemes.length,
    TERMINAL_CUSTOM_SCHEME_LIMIT,
  );
  preferences.addTerminalSchemes(next);
}

function remove(): void {
  if (removing.value) {
    preferences.removeTerminalScheme(removing.value.id);
  }
  removing.value = null;
}

function swatches(colors: readonly string[]): readonly string[] {
  return colors.slice(0, 16);
}

// -- The terminal's own text colour ------------------------------------------

type PaletteTone = "dark" | "light";

const TONES: readonly PaletteTone[] = ["dark", "light"];

/**
 * Whether the palette supplies the text this setting changes.
 *
 * It does when no scheme is worn, and when a scheme is worn for its ANSI colours alone — the two
 * modes where the pane's text is the window's text. A scheme that replaces the whole look brought
 * a text colour of its own, and a picker here would be a control the pane ignores.
 */
const paletteSuppliesText = computed(
  () => preferences.terminalSchemeSource === "host" || preferences.terminalSchemeBackground === "host",
);

/**
 * The palette's own text colour for each half, as `#rrggbb` the colour picker can show.
 *
 * Read from the document rather than from a table of palettes: the stylesheet is where a palette is
 * defined, and the tokens carry both halves at once — the one on screen and the one the terminal
 * would wear if its own theme changed.
 */
function paletteForeground(tone: PaletteTone): string {
  const fallback = tone === "dark" ? "#d8d8d8" : "#222222";
  if (typeof document === "undefined") {
    return fallback;
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(`--terminal-${tone}-foreground`).trim();
  if (raw === "" || typeof CSS === "undefined" || !CSS.supports("color", raw)) {
    return fallback;
  }
  const probe = document.createElement("canvas").getContext("2d");
  if (!probe) {
    return fallback;
  }
  probe.fillStyle = raw;
  const parsed = probe.fillStyle;
  return typeof parsed === "string" && /^#[0-9a-f]{6}$/i.test(parsed) ? parsed : fallback;
}

/** Recomputed when the palette changes, so the swatch shows what "follow" currently means. */
const paletteText = computed<Record<PaletteTone, string>>(() => {
  void preferences.wornPalette;
  void preferences.resolvedTheme;
  return { dark: paletteForeground("dark"), light: paletteForeground("light") };
});

function chosenForeground(tone: PaletteTone): string | null {
  return tone === "dark" ? preferences.terminalForegroundDark : preferences.terminalForegroundLight;
}

function setChosenForeground(tone: PaletteTone, colour: string): void {
  preferences.setTerminalForeground(tone, colour.toLowerCase());
}

function clearChosenForeground(tone: PaletteTone): void {
  preferences.setTerminalForeground(tone, null);
}

/**
 * A typed colour code, or nothing when it is not one.
 *
 * The short form is expanded rather than refused: `#e61919` and `#e619`-style shorthand are the
 * same colour, and somebody pasting one from a stylesheet should not have to rewrite it. Everything
 * else — a half-typed value, a name, a missing digit — answers nothing.
 */
function parseHexColour(value: string): string | null {
  const digits = value.trim().replace(/^#/, "");
  if (!/^(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(digits)) {
    return null;
  }
  const full = digits.length === 3 ? [...digits].map((digit) => digit + digit).join("") : digits;
  return `#${full.toLowerCase()}`;
}

/**
 * Takes what was typed into the colour code field.
 *
 * An empty field means "follow the palette again", which is the one way to write that answer down
 * from the field itself. Anything that is not a colour is put back rather than turned into one:
 * the field shows the colour the terminal is actually drawing.
 */
function commitForegroundCode(tone: PaletteTone, input: HTMLInputElement): void {
  if (input.value.trim() === "") {
    clearChosenForeground(tone);
    input.value = paletteText.value[tone];
    return;
  }
  const parsed = parseHexColour(input.value);
  if (parsed === null) {
    input.value = chosenForeground(tone) ?? paletteText.value[tone];
    return;
  }
  setChosenForeground(tone, parsed);
  input.value = parsed;
}
</script>

<template>
  <div class="space-y-3" data-terminal-schemes>
    <div class="space-y-2 border-b border-border pb-3">
      <span class="text-xs font-medium text-muted-foreground">{{ t("settings.schemes.saved") }}</span>
      <ul class="space-y-1">
        <li
          v-for="entry in profiles"
          :key="entry.id"
          class="flex min-w-0 items-center gap-1 rounded-md border border-border px-1.5 py-1"
          :class="preferences.matchesTerminalSchemeProfile(entry.profile) ? 'border-primary/40 bg-primary/10' : ''"
        >
          <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-2 rounded-sm px-1 py-0.5 text-left text-xs transition-colors hover:text-primary"
            :aria-label="entry.name"
            @click="preferences.applyTerminalSchemeProfile(entry.profile)"
          >
            <span v-if="profileSwatches(entry.profile).length > 0" class="flex shrink-0 gap-px" aria-hidden="true">
              <i
                v-for="(colour, index) in profileSwatches(entry.profile)"
                :key="index"
                class="block h-3.5 w-1.5 rounded-[1px]"
                :style="{ background: colour }"
              />
            </span>
            <span class="min-w-0 flex-1 truncate">{{ entry.name }}</span>
            <span
              v-if="preferences.matchesTerminalSchemeProfile(entry.profile)"
              class="shrink-0 rounded-full border border-primary/40 px-1.5 py-0.5 text-[11px] text-primary"
            >
              {{ t("settings.schemes.worn") }}
            </span>
            <span v-else class="shrink-0 text-[11px] text-muted-foreground">{{ t("settings.schemes.use") }}</span>
          </button>
          <button
            v-if="!entry.builtin"
            type="button"
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
            :title="t('settings.schemes.profileRemove.action')"
            :aria-label="t('settings.schemes.profileRemove.action')"
            @click="removingProfile = entry.profile"
          >
            <Trash2 :size="13" />
          </button>
        </li>
      </ul>
      <div class="flex flex-wrap items-center gap-2">
        <input
          v-model="profileName"
          type="text"
          spellcheck="false"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2.5 py-1.5 text-xs focus:border-primary focus:outline-none"
          :placeholder="t('settings.schemes.savePlaceholder')"
          :aria-label="t('settings.schemes.savePlaceholder')"
          :maxlength="60"
          @keydown.enter.prevent="saveProfile()"
        />
        <button
          type="button"
          class="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="
            profileName.trim() === '' || preferences.terminalSchemeProfiles.length >= TERMINAL_SCHEME_PROFILE_LIMIT
          "
          @click="saveProfile()"
        >
          {{ t("settings.schemes.save") }}
        </button>
      </div>
    </div>

    <div class="grid min-w-0 gap-2 sm:grid-cols-2" role="radiogroup" :aria-label="t('settings.schemes.source')">
      <button
        v-for="option in sources"
        :key="option.value"
        type="button"
        role="radio"
        :aria-checked="preferences.terminalSchemeSource === option.value"
        :class="
          preferences.terminalSchemeSource === option.value
            ? 'rounded-md border border-primary/40 bg-primary/10 p-3 text-left ring-1 ring-primary/30'
            : 'rounded-md border border-border p-3 text-left transition-colors hover:bg-accent'
        "
        @click="preferences.setTerminalSchemeSource(option.value)"
      >
        <span class="block text-sm font-medium">{{ t(option.labelKey) }}</span>
        <span class="mt-0.5 block text-xs text-muted-foreground">{{ t(option.hintKey) }}</span>
      </button>
    </div>

    <div v-if="preferences.terminalSchemeSource === 'custom'" class="space-y-3 pt-1">
      <TerminalSchemePicker
        :model-value="preferences.terminalSchemeDark"
        :title="t('settings.schemes.darkSlot')"
        tone="dark"
        :schemes="schemes"
        @update:model-value="preferences.setTerminalSchemeDark"
      />
      <TerminalSchemePicker
        :model-value="preferences.terminalSchemeLight"
        :title="t('settings.schemes.lightSlot')"
        tone="light"
        :schemes="schemes"
        @update:model-value="preferences.setTerminalSchemeLight"
      />

      <div class="space-y-1.5">
        <span class="block text-xs font-medium text-muted-foreground">{{ t("settings.schemes.background") }}</span>
        <div class="flex flex-wrap gap-2" role="radiogroup" :aria-label="t('settings.schemes.background')">
          <button
            v-for="option in backgrounds"
            :key="option.value"
            type="button"
            role="radio"
            :aria-checked="preferences.terminalSchemeBackground === option.value"
            :class="
              preferences.terminalSchemeBackground === option.value
                ? 'rounded-md border border-primary/40 bg-primary/10 px-3 py-1.5 text-sm text-primary ring-1 ring-primary/30'
                : 'rounded-md border border-border px-3 py-1.5 text-sm transition-colors hover:bg-accent'
            "
            @click="preferences.setTerminalSchemeBackground(option.value)"
          >
            {{ t(option.labelKey) }}
          </button>
        </div>
        <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("settings.schemes.backgroundHint") }}</p>
      </div>

      <div class="space-y-2 border-t border-border pt-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="text-xs font-medium text-muted-foreground">
            {{ t("settings.schemes.imported", { count: imported.length, limit: TERMINAL_CUSTOM_SCHEME_LIMIT }) }}
          </span>
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="imported.length >= TERMINAL_CUSTOM_SCHEME_LIMIT"
            @click="importing = true"
          >
            <Plus :size="12" />
            {{ t("settings.schemes.import.action") }}
          </button>
        </div>

        <ul v-if="imported.length > 0" class="space-y-1">
          <li
            v-for="scheme in imported"
            :key="scheme.id"
            class="flex min-w-0 items-center gap-2 rounded-md border border-border px-2 py-1.5"
          >
            <span class="flex shrink-0 gap-px" aria-hidden="true">
              <i
                v-for="(colour, index) in swatches(scheme.colors)"
                :key="index"
                class="block h-3.5 w-1.5 rounded-[1px]"
                :style="{ background: colour }"
              />
            </span>
            <span class="min-w-0 flex-1 truncate text-xs">{{ scheme.name }}</span>
            <button
              type="button"
              class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
              :title="t('settings.schemes.remove.action')"
              :aria-label="t('settings.schemes.remove.action')"
              @click="removing = scheme"
            >
              <Trash2 :size="13" />
            </button>
          </li>
        </ul>
        <p v-else class="text-[11px] text-muted-foreground">{{ t("settings.schemes.import.none") }}</p>
      </div>
    </div>

    <!--
      The one colour that can be changed without leaving the palette: somebody who likes the
      window's colours — its dark theme especially — and only wants the terminal's text to be
      something else. Shown exactly where the palette supplies the text: with no scheme, and with a
      scheme that replaces only the ANSI colours.
    -->
    <div v-if="paletteSuppliesText" class="space-y-2 border-t border-border pt-3" data-terminal-foreground>
      <div>
        <span class="block text-xs font-medium text-muted-foreground">
          {{ t("settings.schemes.foreground.title") }}
        </span>
        <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
          {{ t("settings.schemes.foreground.hint") }}
        </p>
      </div>
      <div class="grid gap-2 sm:grid-cols-2">
        <div
          v-for="tone in TONES"
          :key="tone"
          class="flex min-w-0 items-center justify-between gap-2 rounded-md border border-border px-2.5 py-1.5"
        >
          <div class="min-w-0">
            <span class="block text-xs font-medium">{{ t(`settings.schemes.foreground.${tone}`) }}</span>
            <span class="block truncate font-mono text-[11px] text-muted-foreground">
              {{ chosenForeground(tone) ?? t("settings.schemes.foreground.follow") }}
            </span>
          </div>
          <div class="flex shrink-0 items-center gap-1">
            <input
              type="color"
              class="h-6 w-9 shrink-0 cursor-pointer rounded border border-border bg-transparent p-0.5"
              :value="chosenForeground(tone) ?? paletteText[tone]"
              :aria-label="t(`settings.schemes.foreground.${tone}`)"
              @input="setChosenForeground(tone, ($event.target as HTMLInputElement).value)"
            />
            <!--
              The code beside the swatch: a colour is sometimes known as one — copied out of a
              stylesheet or a config file — and opening a picker to reproduce it is a detour.
            -->
            <input
              type="text"
              spellcheck="false"
              class="w-20 rounded-sm border border-border bg-background px-1.5 py-0.5 font-mono text-[11px] text-foreground focus:border-primary focus:outline-none"
              :value="chosenForeground(tone) ?? paletteText[tone]"
              :aria-label="t('settings.schemes.foreground.code')"
              @change="commitForegroundCode(tone, $event.target as HTMLInputElement)"
              @keydown.enter.prevent="commitForegroundCode(tone, $event.target as HTMLInputElement)"
            />
            <button
              v-if="chosenForeground(tone)"
              type="button"
              class="grid size-6 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('settings.schemes.foreground.reset')"
              :aria-label="t('settings.schemes.foreground.reset')"
              @click="clearChosenForeground(tone)"
            >
              <RotateCcw :size="12" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <TerminalSchemeImportDialog :open="importing" @update:open="importing = $event" @import="takeImported" />
    <ConfirmDialog
      :open="removing !== null"
      :title="t('settings.schemes.remove.title')"
      :message="t('settings.schemes.remove.message', { name: removing?.name ?? '' })"
      :confirm-label="t('settings.schemes.remove.confirm')"
      danger
      @confirm="remove()"
      @update:open="(open: boolean) => (!open ? (removing = null) : undefined)"
    />
    <ConfirmDialog
      :open="removingProfile !== null"
      :title="t('settings.schemes.profileRemove.title')"
      :message="t('settings.schemes.profileRemove.message', { name: removingProfile?.name ?? '' })"
      :confirm-label="t('settings.schemes.profileRemove.confirm')"
      danger
      @confirm="removeProfile()"
      @update:open="(open: boolean) => (!open ? (removingProfile = null) : undefined)"
    />
  </div>
</template>
