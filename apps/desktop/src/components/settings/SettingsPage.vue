<script setup lang="ts">
/**
 * Settings, as a page in the workspace rather than a dialog over it.
 *
 * A page because a preference is looked at while the thing it is about is still on screen: the
 * theme, the language and the terminal's palette all apply the moment they are chosen, and the tab
 * beside this one is where that is seen. It is also where the settings will keep being added —
 * each new one is a row in `rows.ts` and nothing else.
 *
 * The page is two columns. The categories run down the left for the whole height of the page, and
 * everything else is the column beside them: the search at the top of it, the category's settings
 * under that. The search belongs to that column rather than to the page, because it narrows what is
 * under it — the categories beside it are not filtered by it, they are the other way of getting to
 * the same rows, and they say how many hits each one holds while a search is on.
 *
 * Three things sit above the rows on purpose:
 *
 * 1. **The search.** It looks through every category rather than the one on screen, because
 *    somebody who knows the name of a setting does not know which category it ended up in. A hit
 *    is a row that stays drawn where it is, grouped under its category, so the answer is also the
 *    page it came from.
 * 2. **The categories**, in the order `rows.ts` lists them. One is drawn at a time; the search
 *    temporarily replaces the body with its results.
 * 3. **Restoring the defaults**, per category: it is the one action that is about a whole group
 *    rather than a single row, and doing it per category is what keeps it from being a button that
 *    silently changes six things at once.
 *
 * A setting is drawn in one of two shapes, decided by `rows.ts` rather than by the category it is
 * in: a card of its own, or a field in the grid at the top of the body. Both are the same row of the
 * same table, so the search finds either kind and "restore the defaults" puts either kind back.
 *
 * A write that failed is shown at the top of the body rather than swallowed: the store applies a
 * choice before it writes it, so a failure means what is on screen and what is on disk disagree —
 * which is exactly the thing a settings page should not be quiet about.
 */
import { RotateCcw, Search, X } from "@lucide/vue";
import { TooltipProvider } from "reka-ui";
import { computed, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";

import SettingsControl from "@/components/settings/SettingsControl.vue";
import SettingsField from "@/components/settings/SettingsField.vue";
import SettingsRow from "@/components/settings/SettingsRow.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import KnownKeys from "@/components/settings/KnownKeys.vue";
import TrustedKeys from "@/components/settings/TrustedKeys.vue";
import {
  CUSTOM_COLOR_LABEL_KEYS,
  SETTINGS_CATEGORIES,
  SETTINGS_CATEGORY_LABEL_KEYS,
  SETTINGS_ROWS,
  cardRowsOfCategory,
  fieldRowsOfCategory,
  type SettingsCategoryId,
  type SettingsRowDefinition,
  rowMatches,
  rowsOfCategory,
} from "@/components/settings/rows";
import { createDelayedPreview } from "@/lib/settings/delayedPreview";
import { describeShellError } from "@/i18n/errors";
import type { SupportedLocale } from "@/i18n";
import { useAppStore } from "@/stores/app";
import { useTerminalsStore } from "@/stores/terminals";
import { type PreferenceField, usePreferencesStore } from "@/stores/preferences";
import {
  CUSTOM_COLOR_KEYS,
  DEFAULT_TERMINAL_FONT,
  DEFAULT_UI_FONT,
  type AppLayout,
  type BackgroundDisplayMode,
  type CornerStyle,
  type TabGroupMode,
  type TabLayout,
  type TabMaxWidth,
  type TabPlacement,
  type TabSortMode,
  type HomeLayout,
  type CustomColors,
  type FilesLayout,
  type SessionLayout,
  type ThemeMode,
  type ThemePalette,
} from "@/types/app";

const { t } = useI18n();
const app = useAppStore();
const terminals = useTerminalsStore();
const preferences = usePreferencesStore();

const query = ref("");
const category = ref<SettingsCategoryId>("appearance");

/** The settings this category draws as fields, and the ones it draws as cards under them. */
const fieldRows = computed(() => fieldRowsOfCategory(category.value));
const cardRows = computed(() => cardRowsOfCategory(category.value));

/**
 * The fields of the two grids the appearance page opens with.
 *
 * A row of three for the settings that are a word and a control, and a row of two for the fonts: a
 * family name is longer than the words beside it, and the picker that opens from it is a list of
 * names — so the two share a full row rather than sitting in two thirds of one with a hole in it.
 */
const settingsFields = computed(() =>
  fieldRows.value.filter((row) => row.grid === undefined || row.grid === "settings"),
);
const fontFields = computed(() => fieldRows.value.filter((row) => row.grid === "fonts"));
/**
 * The block a page is drawn around, before everything else on it.
 *
 * The terminal's own preview is the one row that uses it: what is under the preview is about what the
 * preview looks like, so reading the page downwards means meeting it first.
 */
const topFields = computed(() => fieldRows.value.filter((row) => row.grid === "top"));

/** The fields that are a block rather than a word: one to a line, label above. */
const wideFields = computed(() => fieldRows.value.filter((row) => row.grid === "wide"));
/**
 * The four the tab bar is drawn by, in one row, each with the line that says what it is for.
 *
 * The only grid whose fields draw a summary: at a quarter of the page's width there is room for a
 * control and one line under it, and four controls alone would be four values nobody can tell apart.
 * The explanation behind the question mark is there as it is everywhere else.
 */
const tabFields = computed(() => fieldRows.value.filter((row) => row.grid === "four"));
/**
 * The cards that belong under everything else: the two the shell's own window obeys.
 *
 * Split from the other cards rather than reordered in the table, because a card's place on this page
 * is a property of the page: the ones above are what the page is drawn *by*, and these two are what
 * it is drawn *in*.
 */
const trailingCards = computed(() => cardRows.value.filter((row) => row.grid === "end"));
const leadingCards = computed(() => cardRows.value.filter((row) => row.grid !== "end"));

/**
 * The five colours being edited, when a custom palette is worn.
 *
 * Two answers in one: *whether* the pickers are drawn at all, and *which* set they are about. A
 * hand-picked palette is two palettes that share a name, one per theme, so the pickers follow the
 * theme on screen rather than a choice of their own — a light page showing the dark set's colours
 * would be a form about something nobody can see.
 */
const editingCustomColors = computed<CustomColors | null>(() =>
  preferences.wornPalette === "custom"
    ? preferences.resolvedTheme === "dark"
      ? preferences.customColorsDark
      : preferences.customColors
    : null,
);

/** Which set the pickers are about, named in the heading above them. */
const customThemeKey = computed(() => (preferences.resolvedTheme === "dark" ? "editingDark" : "editingLight"));

const searching = computed(() => query.value.trim() !== "");

/** The rows the search is about, in table order; empty while there is no search. */
const hits = computed(() =>
  searching.value ? SETTINGS_ROWS.filter((row) => rowMatches(row, query.value, (key) => t(key))) : [],
);

/** The hits, grouped under the category each one came from — the page it would be found on. */
const groups = computed(() =>
  SETTINGS_CATEGORIES.map((id) => ({ id, rows: hits.value.filter((row) => row.category === id) })).filter(
    (group) => group.rows.length > 0,
  ),
);

function hitsIn(id: SettingsCategoryId): number {
  return hits.value.filter((row) => row.category === id).length;
}

/** Going to a category is also leaving the search: the two are two answers to "what is drawn". */
function selectCategory(id: SettingsCategoryId): void {
  query.value = "";
  category.value = id;
}

/**
 * The fields a category is made of.
 *
 * Three of them have no row — the file panel's height, the sidebar's width, and whether the sidebar
 * is on screen — because nobody chooses them in a row: they are a divider somebody drags and a
 * button in the sidebar's own header. "Restore the defaults" still has to put the frame back the
 * way it came, so the category that owns them names them here.
 */
function fieldsOf(id: SettingsCategoryId): readonly PreferenceField[] {
  // Rows that write nothing — the terminal's preview — are not among the fields a category restores.
  const fields: PreferenceField[] = rowsOfCategory(id).flatMap((row) => (row.field ? [row.field] : []));
  if (id === "workspace") {
    fields.push("terminalPanelHeight", "sidebarWidth", "sidebarOpen");
  }
  if (id === "appearance") {
    fields.push("customColors", "customColorsDark");
  }
  return fields;
}

/** What a row's control currently shows. */
function currentValue(row: SettingsRowDefinition): string | boolean | readonly string[] {
  // A row that writes nothing has no value: the control it draws is a picture of the settings around
  // it, and it reads what it needs from the store itself.
  if (!row.field) {
    return "";
  }
  switch (row.field) {
    case "themeMode":
      return preferences.themeMode;
    case "themePalette":
      return preferences.themePalette;
    case "uiScale":
      // A number in the store and a string in a dropdown: the value the settings file holds is the
      // number, so the conversion is here rather than in the store, which keeps it a number.
      return String(preferences.uiScale);
    case "cornerStyle":
      return preferences.cornerStyle;
    case "homeLayout":
      return preferences.homeLayout;
    case "appLayout":
      return preferences.appLayout;
    case "tabLayout":
      return preferences.tabLayout;
    case "tabPlacement":
      return preferences.tabPlacement;
    case "tabGroupMode":
      return preferences.tabGroupMode;
    case "tabSortMode":
      return preferences.tabSortMode;
    case "tabMaxWidth":
      return preferences.tabMaxWidth;
    case "backgroundImage":
      // The block reads and writes the picture itself, because it is five answers to one question and
      // only one of them — the mode — is a value this page's contract could hand over.
      return preferences.backgroundImage.displayMode;
    case "terminalFontSize":
      // A number in the store and a string through this contract, for the same reason the interface
      // scale is: the value the settings file holds is a number, and the slider is the only thing here
      // that reads it as text.
      return String(preferences.terminalFontSize);
    case "showTrayIcon":
      return preferences.showTrayIcon;
    case "quitOnClose":
      return preferences.quitOnClose;
    case "uiFont":
      // Nothing chosen reads as the one this build ships with, because that is what is being drawn:
      // the picker shows a family, and "no answer" is not one.
      return preferences.uiFont ?? DEFAULT_UI_FONT;
    case "terminalFont":
      return preferences.terminalFont ?? DEFAULT_TERMINAL_FONT;
    case "terminalTheme":
      return preferences.terminalTheme;
    case "locale":
      return preferences.locale;
    case "sessionLayout":
      return preferences.sessionLayout;
    case "monitorEnabled":
      return preferences.monitorEnabled;
    case "filesLayout":
      return preferences.filesLayout;
    case "packedTransfer":
      return preferences.packedTransfer;
  }
}

/**
 * Writing a row.
 *
 * It applies at once, and it is written down when the bar at the foot of the page says so — which is
 * the difference the two buttons there are about.
 */
function choose(row: SettingsRowDefinition, value: string | boolean | readonly string[]): void {
  if (!row.field) {
    return;
  }
  switch (row.field) {
    case "themeMode":
      void preferences.setThemeMode(value as ThemeMode);
      return;
    case "themePalette":
      void preferences.setThemePalette(value as ThemePalette);
      return;
    case "uiScale":
      void preferences.setUiScale(Number(value));
      return;
    case "cornerStyle":
      void preferences.setCornerStyle(value as CornerStyle);
      return;
    case "homeLayout":
      void preferences.setHomeLayout(value as HomeLayout);
      return;
    case "appLayout":
      void preferences.setAppLayout(value as AppLayout);
      return;
    case "tabLayout":
      void preferences.setTabLayout(value as TabLayout);
      return;
    case "tabPlacement":
      void preferences.setTabPlacement(value as TabPlacement);
      return;
    case "tabGroupMode":
      void preferences.setTabGroupMode(value as TabGroupMode);
      return;
    case "tabSortMode":
      void preferences.setTabSortMode(value as TabSortMode);
      return;
    case "tabMaxWidth":
      void preferences.setTabMaxWidth(value as TabMaxWidth);
      return;
    case "backgroundImage":
      void preferences.setBackgroundDisplayMode(value as BackgroundDisplayMode);
      return;
    case "terminalFontSize":
      preferences.setTerminalFontSize(Number(value));
      return;
    case "showTrayIcon":
      preferences.setShowTrayIcon(value === true);
      return;
    case "quitOnClose":
      preferences.setQuitOnClose(value === true);
      return;
    case "uiFont":
      void preferences.setUiFont(String(value));
      return;
    case "terminalFont":
      void preferences.setTerminalFont(String(value));
      return;
    case "terminalTheme":
      void preferences.setTerminalTheme(value as ThemeMode);
      return;
    case "locale":
      void preferences.setLocale(value as SupportedLocale);
      return;
    case "sessionLayout":
      void preferences.setSessionLayout(value as SessionLayout);
      return;
    case "monitorEnabled":
      void preferences.setMonitorEnabled(value === true);
      return;
    case "filesLayout":
      void preferences.setFilesLayout(value as FilesLayout);
      return;
    case "packedTransfer":
      void preferences.setPackedTransfer(value === true);
      return;
  }
}

/**
 * How long the pointer rests on a colour before it is shown.
 *
 * Short enough to read as instant, long enough that crossing the list on the way to another option
 * does not repaint the window on every one it passes.
 */
const PREVIEW_DELAY_MS = 80;

/**
 * Showing a setting's option before it is taken.
 *
 * Only a colour has anything to show: every other setting here is a word, and a word says what it
 * is without being worn. A colour is the one whose whole content is what it looks like, so it is
 * the one that can be looked at before it is chosen — nothing is written, and letting go of the
 * list puts back whatever was actually chosen.
 */
const showPaletteOption = createDelayedPreview<string>((palette) => {
  preferences.previewThemePalette(palette as ThemePalette);
}, PREVIEW_DELAY_MS);

/**
 * The same delay for the two font lists, which are the other settings whose content is what they
 * look like. Which of the two is being previewed travels with the family, because dressing the
 * terminal's list in the interface's font — or the other way round — is not a preview of anything.
 */
const showFontOption = createDelayedPreview<{ field: "uiFont" | "terminalFont"; value: string }>(
  ({ field, value }) => preferences.previewFont(field, value),
  PREVIEW_DELAY_MS,
);

function preview(row: SettingsRowDefinition, value: string): void {
  if (row.field === "themePalette") {
    showPaletteOption.schedule(value);
  }
  if (row.field === "uiFont" || row.field === "terminalFont") {
    showFontOption.schedule({ field: row.field, value });
  }
}

function endPreview(row: SettingsRowDefinition): void {
  if (row.field === "themePalette") {
    showPaletteOption.cancel();
    preferences.clearThemePalettePreview();
  }
  if (row.field === "uiFont" || row.field === "terminalFont") {
    showFontOption.cancel();
    preferences.clearFontPreview();
  }
}

/**
 * What the bar at the foot of the page does.
 *
 * A choice is applied the moment it is made — a colour nobody can see before choosing it is not a
 * choice — and the settings *file* is written when somebody says so. That is the difference the two
 * buttons on the right are about, and it is why the colour picker no longer writes the file on every
 * mouse move of a drag.
 *
 * Leaving is the awkward one, and it has an answer rather than a rule: the tab can be closed from
 * four places (this bar, its own cross, the tab's menu, the window), so every route out has to keep
 * what was chosen rather than drop it silently. This page therefore writes on its way out — and the
 * one way out that *does* throw something away is the Close button here, which asks first.
 */
const leaving = ref(false);

/** Puts the category on screen back the way a fresh install has it. */
function restoreDefaults(): void {
  void preferences.reset(fieldsOf(category.value));
}

/** Writes down what is on screen. Nothing changes; the file catches up. */
function apply(): void {
  void preferences.commit();
}

async function applyAndClose(): Promise<void> {
  if (await preferences.commit()) {
    await terminals.closeSettings();
  }
}

function askToLeave(): void {
  if (!preferences.dirty) {
    void terminals.closeSettings();
    return;
  }
  leaving.value = true;
}

async function writeAndClose(): Promise<void> {
  // The close only happens if the write did: a page that vanished with the choices still unwritten
  // would be the one thing this bar exists to prevent.
  await applyAndClose();
  leaving.value = false;
}

async function discardAndClose(): Promise<void> {
  await preferences.discard();
  leaving.value = false;
  await terminals.closeSettings();
}

/**
 * A page that is closed mid-look leaves nothing behind.
 *
 * The list is drawn outside this component, so closing the tab while an option is under the pointer
 * is a way out of the page that never reaches `endPreview` — and the window would go on wearing a
 * colour nobody can see the reason for, since the list it came from is gone.
 */
onBeforeUnmount(() => {
  preferences.clearThemePalettePreview();
  preferences.clearFontPreview();
  // The window is showing choices the file has not been told about, and this page is going away: the
  // point of applying as you go is that nothing is lost, so the last thing the page does is write.
  if (preferences.dirty) {
    void preferences.commit();
  }
});

/**
 * What the About category lists: the build's own facts, which nobody chooses and nobody restores.
 *
 * The value is read off `app.info`, which is what the backend answered at start-up — so an
 * interface that has not been told yet draws the loading line rather than empty rows. The keys are
 * the home screen's, because these are the same three facts it has always printed.
 */
const aboutFacts = computed(() =>
  app.info
    ? [
        { id: "version", labelKey: "home.about.version", value: app.info.version },
        { id: "platform", labelKey: "home.about.platform", value: `${app.info.os} ${app.info.arch}` },
        {
          id: "descriptor-schema",
          labelKey: "home.about.descriptorSchema",
          value: String(app.info.descriptorSchemaVersion),
        },
      ]
    : [],
);

const problem = computed(() => (preferences.problem ? describeShellError(translate, preferences.problem) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <!--
    One provider for the page rather than one per row: the open delay is a property of the surface
    the tooltips are on, and a question mark that takes most of a second to explain itself is worse
    than one that answers on hover.
  -->
  <TooltipProvider :delay-duration="200">
    <!--
      The page is chrome: a drag across it means reaching for a control, not taking a reading away,
      so the selection is refused once here — the same answer the monitor panel gives, and for the
      same reason. The readings a page of settings does hold are the ones exempted below: what the
      build is, the keys this device holds and trusts, an error somebody has to quote, and the two
      fields somebody types in.
    -->
    <section class="@container flex h-full min-h-0 flex-col select-none" :aria-label="t('settings.title')">
      <div class="flex min-h-0 flex-1">
        <nav
          class="flex w-44 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-2"
          :aria-label="t('settings.nav.label')"
        >
          <button
            v-for="id in SETTINGS_CATEGORIES"
            :key="id"
            type="button"
            class="flex w-full items-center justify-between gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors"
            :class="
              !searching && category === id
                ? 'bg-primary text-primary-foreground shadow-sm'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            "
            :aria-current="!searching && category === id ? 'true' : undefined"
            @click="selectCategory(id)"
          >
            <span class="truncate">{{ t(SETTINGS_CATEGORY_LABEL_KEYS[id]) }}</span>
            <!-- While searching, the way to a hit is the category it is in — so the count is what
                 tells apart a category that has one from a category that does not. -->
            <span v-if="searching && hitsIn(id) > 0" class="shrink-0 font-mono text-[11px] opacity-70">
              {{ hitsIn(id) }}
            </span>
          </button>
        </nav>

        <div class="flex min-w-0 flex-1 flex-col">
          <!--
            The search is drawn at the top of the column the settings are read in, not across the
            whole page above the categories: it narrows what is under it, and the categories beside
            it are the other way to reach the same rows. The whole column is the field's, because
            the answer to a search is a page of results rather than a list hanging under a box.
          -->
          <div class="shrink-0 px-5 pt-4 pb-3">
            <div class="relative mx-auto w-full max-w-3xl">
              <Search
                :size="14"
                class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted-foreground"
              />
              <!--
                A real search field, so the role comes from the markup rather than from an override —
                with the platform's own clear button turned off in the stylesheet, because this
                window draws that button itself and a second one in the same field would be one too
                many.
              -->
              <input
                v-model="query"
                type="search"
                class="h-10 w-full rounded-lg border border-border bg-muted/30 pr-8 pl-9 text-sm transition-colors select-text hover:bg-muted/40 focus-visible:border-primary focus-visible:bg-background focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none"
                :placeholder="t('settings.search.placeholder')"
                :aria-label="t('settings.search.placeholder')"
                @keydown.esc="query = ''"
              />
              <button
                v-if="searching"
                type="button"
                class="absolute top-1/2 right-2 grid size-6 -translate-y-1/2 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :aria-label="t('settings.search.clear')"
                @click="query = ''"
              >
                <X :size="13" />
              </button>
            </div>
          </div>

          <div class="min-h-0 flex-1 overflow-y-auto">
            <div class="mx-auto w-full max-w-3xl space-y-6 px-5 pb-5">
              <!-- An error is quoted somewhere, so it is the page's text and not the page's chrome. -->
              <p
                v-if="problem"
                data-settings-problem
                class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
              >
                <span>{{ problem.message }}</span>
                <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
              </p>

              <template v-if="searching">
                <section v-for="group in groups" :key="group.id" class="space-y-2">
                  <h3 class="text-sm font-medium text-muted-foreground">
                    {{ t(SETTINGS_CATEGORY_LABEL_KEYS[group.id]) }}
                  </h3>
                  <SettingsRow
                    v-for="row in group.rows"
                    :key="row.id"
                    :title="t(row.titleKey)"
                    :summary="t(row.summaryKey)"
                    :help="t(row.helpKey)"
                    found
                  >
                    <SettingsControl
                      :row="row"
                      :value="currentValue(row)"
                      :label="t(row.titleKey)"
                      @choose="(value) => choose(row, value)"
                      @preview="(value: string) => preview(row, value)"
                      @end-preview="endPreview(row)"
                    />
                  </SettingsRow>
                </section>

                <p
                  v-if="groups.length === 0"
                  class="rounded-lg border border-dashed border-border px-4 py-12 text-center text-sm text-muted-foreground"
                >
                  {{ t("settings.search.empty", { query: query.trim() }) }}
                </p>
              </template>

              <!--
                No heading for the category: the navigation beside this column already says which one
                is on screen, in the item that is lit — and a page that repeats its own name under the
                thing that named it has said nothing twice. The search is the other case and keeps its
                headings: a hit is grouped under the category it came from, which is the answer to
                "where is this setting".
              -->
              <section v-else class="space-y-2">
                <template v-if="category === 'about'">
                  <!-- The build's own facts: a version and a platform are pasted into bug reports. -->
                  <dl v-if="aboutFacts.length > 0" class="space-y-2 select-text">
                    <div
                      v-for="fact in aboutFacts"
                      :key="fact.id"
                      class="flex items-center justify-between gap-4 rounded-md border border-border bg-muted/20 px-3 py-2.5"
                    >
                      <dt class="text-sm font-medium">{{ t(fact.labelKey) }}</dt>
                      <dd class="font-mono text-[11px] text-muted-foreground">{{ fact.value }}</dd>
                    </div>
                  </dl>
                  <p v-else class="text-xs text-muted-foreground">{{ t("common.status.loading") }}</p>
                </template>

                <!--
                A category that is a list rather than a set of choices: there is nothing in it to
                search for and nothing to restore, so it draws its own content —
                `LIST_CATEGORIES` is the same list this branch answers to.
              -->
                <div v-else-if="category === 'security'" class="space-y-5">
                  <div class="space-y-2">
                    <h4 class="text-sm font-medium text-muted-foreground">{{ t("settings.security.keys") }}</h4>
                    <KnownKeys />
                  </div>
                  <div class="space-y-2">
                    <h4 class="text-sm font-medium text-muted-foreground">{{ t("settings.security.hosts") }}</h4>
                    <TrustedKeys />
                  </div>
                </div>

                <template v-else>
                  <!--
                  The block the page is drawn around, before the fields: what sits under it is about what
                  it looks like.
                -->
                  <div v-if="topFields.length > 0" class="grid gap-y-4 pb-1">
                    <SettingsField
                      v-for="row in topFields"
                      :key="row.id"
                      :title="t(row.titleKey)"
                      :help="t(row.helpKey)"
                    >
                      <SettingsControl
                        :row="row"
                        :value="currentValue(row)"
                        :label="t(row.titleKey)"
                        stretch
                        @choose="(value) => choose(row, value)"
                        @preview="(value: string) => preview(row, value)"
                        @end-preview="endPreview(row)"
                      />
                    </SettingsField>
                  </div>

                  <!--
                  The category's fields first, in one grid: a language, a colour and a size are each
                  a name and a control, and three cards for them would push the settings somebody
                  actually reads down the page. What they give up is the summary line — the question
                  mark still opens the whole explanation, and the search still finds them by either
                  line, because both are in the table rather than in the drawing.
                -->
                  <div v-if="settingsFields.length > 0" class="grid gap-x-3 gap-y-4 pb-1 sm:grid-cols-3">
                    <SettingsField
                      v-for="row in settingsFields"
                      :key="row.id"
                      :title="t(row.titleKey)"
                      :help="t(row.helpKey)"
                    >
                      <SettingsControl
                        :row="row"
                        :value="currentValue(row)"
                        :label="t(row.titleKey)"
                        stretch
                        @choose="(value) => choose(row, value)"
                        @preview="(value: string) => preview(row, value)"
                        @end-preview="endPreview(row)"
                      />
                    </SettingsField>

                    <!--
                    A hand-picked palette is edited in the same grid, one field per colour: the
                    five are the same kind of thing as the three above them — a name and a control —
                    and they appear only once the one that says so has been chosen. Which set they
                    are about is the heading, because a person switching themes has to be able to
                    see which half of their palette they are looking at.
                  -->
                    <template v-if="editingCustomColors">
                      <div class="flex items-end justify-between gap-2 sm:col-span-3">
                        <span class="text-xs text-muted-foreground">
                          {{ t(`settings.custom.${customThemeKey}`) }}
                        </span>
                        <button
                          type="button"
                          class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                          @click="preferences.resetCustomColors()"
                        >
                          <RotateCcw :size="12" />
                          {{ t("settings.custom.reset") }}
                        </button>
                      </div>
                      <div
                        v-for="key in CUSTOM_COLOR_KEYS"
                        :key="key"
                        class="flex min-w-0 items-center justify-between gap-2"
                      >
                        <label :for="`custom-colour-${key}`" class="truncate text-xs font-medium text-muted-foreground">
                          {{ t(CUSTOM_COLOR_LABEL_KEYS[key]) }}
                        </label>
                        <input
                          :id="`custom-colour-${key}`"
                          type="color"
                          class="h-6 w-10 shrink-0 cursor-pointer rounded border border-border bg-transparent p-0.5 select-text"
                          :value="editingCustomColors[key]"
                          @input="
                            preferences.setCustomColor(key, ($event.target as HTMLInputElement).value.toLowerCase())
                          "
                        />
                      </div>
                    </template>
                  </div>

                  <div v-if="fontFields.length > 0" class="grid gap-x-3 gap-y-4 pb-1 sm:grid-cols-2">
                    <SettingsField
                      v-for="row in fontFields"
                      :key="row.id"
                      :title="t(row.titleKey)"
                      :help="t(row.helpKey)"
                    >
                      <SettingsControl
                        :row="row"
                        :value="currentValue(row)"
                        :label="t(row.titleKey)"
                        stretch
                        @choose="(value) => choose(row, value)"
                        @preview="(value: string) => preview(row, value)"
                        @end-preview="endPreview(row)"
                      />
                    </SettingsField>
                  </div>

                  <SettingsRow
                    v-for="row in leadingCards"
                    :key="row.id"
                    :title="t(row.titleKey)"
                    :summary="t(row.summaryKey)"
                    :help="t(row.helpKey)"
                  >
                    <SettingsControl
                      :row="row"
                      :value="currentValue(row)"
                      :label="t(row.titleKey)"
                      @choose="(value) => choose(row, value)"
                      @preview="(value: string) => preview(row, value)"
                      @end-preview="endPreview(row)"
                    />
                  </SettingsRow>

                  <!--
                  The blocks last: a row above is a name and a control, and a block is a shape of its
                  own — two cards to choose between, a label over it rather than beside it. Putting one
                  between the rows would break the column they read down, and the settings a person
                  changes often are the rows, so the block is what they come to after them.
                -->
                  <div v-if="wideFields.length > 0" class="grid gap-y-4 pt-2 pb-1">
                    <SettingsField
                      v-for="row in wideFields"
                      :key="row.id"
                      :title="t(row.titleKey)"
                      :help="t(row.helpKey)"
                    >
                      <SettingsControl
                        :row="row"
                        :value="currentValue(row)"
                        :label="t(row.titleKey)"
                        stretch
                        @choose="(value) => choose(row, value)"
                        @preview="(value: string) => preview(row, value)"
                        @end-preview="endPreview(row)"
                      />
                    </SettingsField>
                  </div>

                  <!--
                  The row of four, last of all: the tab bar's own choices are answers to one
                  question, so they are read together rather than one per screenful — and the bar is
                  the last part of the window somebody looks at before they leave this page.
                -->
                  <div
                    v-if="tabFields.length > 0"
                    class="grid gap-x-3 gap-y-4 pt-2 pb-1 sm:grid-cols-2 @4xl:grid-cols-4"
                  >
                    <SettingsField
                      v-for="row in tabFields"
                      :key="row.id"
                      :title="t(row.titleKey)"
                      :help="t(row.helpKey)"
                      :summary="t(row.summaryKey)"
                    >
                      <SettingsControl
                        :row="row"
                        :value="currentValue(row)"
                        :label="t(row.titleKey)"
                        stretch
                        @choose="(value) => choose(row, value)"
                        @preview="(value: string) => preview(row, value)"
                        @end-preview="endPreview(row)"
                      />
                    </SettingsField>
                  </div>

                  <!--
                  The two the shell's own window obeys: last on the page, because they are not about
                  how anything is drawn, and full-width cards because each one is a sentence — what
                  the switch does, and what happens to the window either way.
                -->
                  <SettingsRow
                    v-for="row in trailingCards"
                    :key="row.id"
                    :title="t(row.titleKey)"
                    :summary="t(row.summaryKey)"
                    :help="t(row.helpKey)"
                  >
                    <SettingsControl
                      :row="row"
                      :value="currentValue(row)"
                      :label="t(row.titleKey)"
                      @choose="(value) => choose(row, value)"
                      @preview="(value: string) => preview(row, value)"
                      @end-preview="endPreview(row)"
                    />
                  </SettingsRow>
                </template>
              </section>
            </div>
          </div>

          <!--
            The bar at the foot of the page, and the only place any of it can be said: restoring a
            category is about the whole group rather than a row, and writing is about the page rather
            than a setting. Reset is on the left because it is about the page's contents, and the
            three on the right are about leaving — the same shape the reference puts them in.
          -->
          <footer class="flex shrink-0 flex-wrap items-center gap-2 border-t border-border px-5 py-3">
            <button
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
              :disabled="fieldsOf(category).length === 0"
              :aria-label="t('settings.reset.for', { category: t(SETTINGS_CATEGORY_LABEL_KEYS[category]) })"
              @click="restoreDefaults()"
            >
              <RotateCcw :size="13" />
              {{ t("settings.reset.label") }}
            </button>

            <span class="flex-1" />

            <button
              type="button"
              class="rounded-md border border-border px-3 py-1.5 text-sm transition-colors hover:bg-accent"
              @click="askToLeave()"
            >
              {{ t("common.action.close") }}
            </button>
            <button
              type="button"
              class="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
              :disabled="!preferences.dirty"
              @click="apply()"
            >
              {{ t("settings.apply") }}
            </button>
            <button
              type="button"
              class="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
              :disabled="!preferences.dirty"
              @click="applyAndClose()"
            >
              {{ t("settings.applyAndClose") }}
            </button>
          </footer>
        </div>
      </div>
    </section>

    <!--
      The one way out that throws something away, and it asks first. Three answers rather than two:
      the choices can be written, dropped, or left where they are — and "Cancel" would have to mean
      one of those without saying which.
    -->
    <ConfirmDialog
      v-if="leaving"
      :open="true"
      :title="t('settings.leaving.title')"
      :message="t('settings.leaving.message')"
      :confirm-label="t('settings.leaving.write')"
      :discard-label="t('settings.leaving.drop')"
      @confirm="writeAndClose()"
      @discard="discardAndClose()"
      @update:open="leaving = false"
    />
  </TooltipProvider>
</template>
