<script setup lang="ts">
/**
 * Settings, as a page in the workspace rather than a dialog over it.
 *
 * A page because a preference is looked at while the thing it is about is still on screen: the
 * theme, the language and the terminal's palette all apply the moment they are chosen, and the tab
 * beside this one is where that is seen. It is also where the settings will keep being added —
 * each new one is a row in `rows.ts` and nothing else.
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
 * A write that failed is shown at the top of the body rather than swallowed: the store applies a
 * choice before it writes it, so a failure means what is on screen and what is on disk disagree —
 * which is exactly the thing a settings page should not be quiet about.
 */
import { RotateCcw, Search, X } from "@lucide/vue";
import { TooltipProvider } from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import SettingsControl from "@/components/settings/SettingsControl.vue";
import SettingsRow from "@/components/settings/SettingsRow.vue";
import KnownKeys from "@/components/settings/KnownKeys.vue";
import TrustedKeys from "@/components/settings/TrustedKeys.vue";
import {
  LIST_CATEGORIES,
  SETTINGS_CATEGORIES,
  SETTINGS_CATEGORY_LABEL_KEYS,
  SETTINGS_ROWS,
  type SettingsCategoryId,
  type SettingsRowDefinition,
  rowMatches,
  rowsOfCategory,
} from "@/components/settings/rows";
import { describeShellError } from "@/i18n/errors";
import type { SupportedLocale } from "@/i18n";
import { useAppStore } from "@/stores/app";
import { type PreferenceField, usePreferencesStore } from "@/stores/preferences";
import type { FilesLayout, SessionLayout, ThemeMode } from "@/types/app";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();

const query = ref("");
const category = ref<SettingsCategoryId>("appearance");

/** Whether the category on screen is a list of things rather than a set of choices to restore. */
const isListCategory = computed(() => LIST_CATEGORIES.includes(category.value));

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

/** The fields a category is made of. The panel height has no row, so the workspace names it here. */
function fieldsOf(id: SettingsCategoryId): readonly PreferenceField[] {
  const fields: PreferenceField[] = rowsOfCategory(id).map((row) => row.field);
  if (id === "workspace") {
    fields.push("terminalPanelHeight");
  }
  return fields;
}

/** What a row's control currently shows. */
function currentValue(row: SettingsRowDefinition): string | boolean {
  switch (row.field) {
    case "themeMode":
      return preferences.themeMode;
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

/** Writing a row, which applies at once and is written down afterwards — there is no save button. */
function choose(row: SettingsRowDefinition, value: string | boolean): void {
  switch (row.field) {
    case "themeMode":
      void preferences.setThemeMode(value as ThemeMode);
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
    <section class="@container flex h-full min-h-0 flex-col" :aria-label="t('settings.title')">
      <header class="flex shrink-0 items-center gap-3 border-b border-border px-3 py-2">
        <div class="relative w-full max-w-sm">
          <Search
            :size="14"
            class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted-foreground"
          />
          <!--
            A real search field, so the role comes from the markup rather than from an override —
            with the platform's own clear button turned off in the stylesheet, because this window
            draws that button itself and a second one in the same field would be one too many.
          -->
          <input
            v-model="query"
            type="search"
            class="h-9 w-full rounded-lg border border-border bg-muted/30 pr-8 pl-9 text-sm transition-colors hover:bg-muted/40 focus-visible:border-primary focus-visible:bg-background focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none"
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
      </header>

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
            <span v-if="searching && hitsIn(id) > 0" class="shrink-0 font-mono text-[10px] opacity-70">
              {{ hitsIn(id) }}
            </span>
          </button>
        </nav>

        <div class="min-h-0 flex-1 overflow-y-auto">
          <div class="mx-auto w-full max-w-3xl space-y-6 px-6 py-5">
            <p v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
              <span>{{ problem.message }}</span>
              <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
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
                    @choose="(value: string | boolean) => choose(row, value)"
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

            <section v-else class="space-y-2">
              <h3 class="text-sm font-medium text-muted-foreground">{{ t(SETTINGS_CATEGORY_LABEL_KEYS[category]) }}</h3>

              <template v-if="category === 'about'">
                <dl v-if="aboutFacts.length > 0" class="space-y-2">
                  <div
                    v-for="fact in aboutFacts"
                    :key="fact.id"
                    class="flex items-center justify-between gap-4 rounded-md border border-border bg-muted/20 px-3 py-2.5"
                  >
                    <dt class="text-sm">{{ t(fact.labelKey) }}</dt>
                    <dd class="font-mono text-xs text-muted-foreground">{{ fact.value }}</dd>
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
                  <h4 class="text-xs font-medium text-muted-foreground">{{ t("settings.security.keys") }}</h4>
                  <KnownKeys />
                </div>
                <div class="space-y-2">
                  <h4 class="text-xs font-medium text-muted-foreground">{{ t("settings.security.hosts") }}</h4>
                  <TrustedKeys />
                </div>
              </div>

              <template v-else>
                <SettingsRow
                  v-for="row in rowsOfCategory(category)"
                  :key="row.id"
                  :title="t(row.titleKey)"
                  :summary="t(row.summaryKey)"
                  :help="t(row.helpKey)"
                >
                  <SettingsControl
                    :row="row"
                    :value="currentValue(row)"
                    :label="t(row.titleKey)"
                    @choose="(value: string | boolean) => choose(row, value)"
                  />
                </SettingsRow>

                <div v-if="!isListCategory" class="flex justify-end pt-1">
                  <button
                    type="button"
                    class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    :aria-label="t('settings.reset.for', { category: t(SETTINGS_CATEGORY_LABEL_KEYS[category]) })"
                    @click="preferences.reset(fieldsOf(category))"
                  >
                    <RotateCcw :size="12" />
                    {{ t("settings.reset.label") }}
                  </button>
                </div>
              </template>
            </section>
          </div>
        </div>
      </div>
    </section>
  </TooltipProvider>
</template>
