<script setup lang="ts">
/**
 * The words the terminal marks, and how it marks them.
 *
 * The rules live in the settings file rather than in a terminal: a host does not decide that
 * "timeout" matters, the person reading it does, and the same list should follow them from tab to
 * tab. The list is bounded because every rule is matched against every new line.
 */
import { Pencil, Plus, Trash2 } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import TerminalHighlightDialog from "@/components/settings/TerminalHighlightDialog.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { usePreferencesStore } from "@/stores/preferences";
import { TERMINAL_HIGHLIGHT_RULE_LIMIT, type TerminalHighlightMode, type TerminalHighlightRule } from "@/types/app";

const { t } = useI18n();
const preferences = usePreferencesStore();

const modes: readonly { value: TerminalHighlightMode; labelKey: string; hintKey: string }[] = [
  {
    value: "background",
    labelKey: "settings.highlights.modeBackground",
    hintKey: "settings.highlights.modeBackgroundHint",
  },
  { value: "text", labelKey: "settings.highlights.modeText", hintKey: "settings.highlights.modeTextHint" },
];

const rules = computed(() => preferences.terminalHighlightRules);
const takenIds = computed(() => rules.value.map((rule) => rule.id));

const editing = ref<TerminalHighlightRule | null>(null);
const dialogOpen = ref(false);
const removing = ref<TerminalHighlightRule | null>(null);

function add(): void {
  editing.value = null;
  dialogOpen.value = true;
}

function edit(rule: TerminalHighlightRule): void {
  editing.value = rule;
  dialogOpen.value = true;
}

function save(rule: TerminalHighlightRule): void {
  preferences.saveTerminalHighlightRule(rule);
}

function remove(): void {
  if (removing.value) {
    preferences.removeTerminalHighlightRule(removing.value.id);
  }
  removing.value = null;
}
</script>

<template>
  <div class="space-y-3" data-terminal-highlights>
    <div class="grid min-w-0 gap-2 sm:grid-cols-2" role="radiogroup" :aria-label="t('settings.highlights.mode')">
      <button
        v-for="option in modes"
        :key="option.value"
        type="button"
        role="radio"
        :aria-checked="preferences.terminalHighlightMode === option.value"
        :class="
          preferences.terminalHighlightMode === option.value
            ? 'rounded-md border border-primary/40 bg-primary/10 p-3 text-left ring-1 ring-primary/30'
            : 'rounded-md border border-border p-3 text-left transition-colors hover:bg-accent'
        "
        @click="preferences.setTerminalHighlightMode(option.value)"
      >
        <span class="block text-sm font-medium">{{ t(option.labelKey) }}</span>
        <span class="mt-0.5 block text-xs text-muted-foreground">{{ t(option.hintKey) }}</span>
      </button>
    </div>

    <div class="space-y-2 border-t border-border pt-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <span class="text-xs font-medium text-muted-foreground">
          {{ t("settings.highlights.rules", { count: rules.length, limit: TERMINAL_HIGHLIGHT_RULE_LIMIT }) }}
        </span>
        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="rules.length >= TERMINAL_HIGHLIGHT_RULE_LIMIT"
          @click="add()"
        >
          <Plus :size="12" />
          {{ t("settings.highlights.add") }}
        </button>
      </div>

      <ul v-if="rules.length > 0" class="space-y-1">
        <li
          v-for="rule in rules"
          :key="rule.id"
          class="flex min-w-0 items-center gap-2 rounded-md border border-border px-2 py-1.5"
        >
          <i class="size-3 shrink-0 rounded-full" :style="{ background: rule.color }" aria-hidden="true"></i>
          <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ rule.pattern }}</span>
          <span
            v-if="rule.isRegex"
            class="shrink-0 rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground"
          >
            {{ t("settings.highlights.regexBadge") }}
          </span>
          <span
            v-if="rule.caseSensitive"
            class="shrink-0 rounded border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground"
          >
            {{ t("settings.highlights.caseBadge") }}
          </span>
          <button
            type="button"
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('settings.highlights.edit')"
            :aria-label="t('settings.highlights.edit')"
            @click="edit(rule)"
          >
            <Pencil :size="12" />
          </button>
          <button
            type="button"
            class="grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
            :title="t('settings.highlights.remove')"
            :aria-label="t('settings.highlights.remove')"
            @click="removing = rule"
          >
            <Trash2 :size="13" />
          </button>
        </li>
      </ul>
      <p v-else class="text-[11px] text-muted-foreground">{{ t("settings.highlights.empty") }}</p>
    </div>

    <TerminalHighlightDialog
      :open="dialogOpen"
      :rule="editing"
      :taken-ids="takenIds"
      @save="save"
      @update:open="dialogOpen = $event"
    />
    <ConfirmDialog
      :open="removing !== null"
      :title="t('settings.highlights.removeTitle')"
      :message="t('settings.highlights.removeMessage', { pattern: removing?.pattern ?? '' })"
      :confirm-label="t('settings.highlights.remove')"
      danger
      @confirm="remove()"
      @update:open="(open: boolean) => (!open ? (removing = null) : undefined)"
    />
  </div>
</template>
