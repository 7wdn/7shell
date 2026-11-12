<script setup lang="ts">
/**
 * The policy matrix: what unattended privileged work may do, and where.
 *
 * The page is a draft. Every control changes a copy of the matrix and nothing else; the Apply at the
 * foot hands the copy to the store and writes the settings file. That is not the shape the rest of
 * the page follows by accident — a matrix is a rule about what an agent or a batch may do, and
 * changing one while typing a host name would change what runs next without anybody having said so.
 *
 * Two facts the page states rather than assumes:
 *
 * 1. **The most restrictive layer wins.** A group, host or directory rule can tighten the global
 *    rule and never loosen it, so the page says so instead of drawing the exceptions as overrides.
 * 2. **Destructive and catastrophic cannot be allowed away.** The resolver raises a matching
 *    `allow` to `ask` for those two levels, so a row that says "allow" there is a statement about
 *    where the question is asked, not whether it is. The note under the global rule says that.
 *
 * Nothing here is a command: the panel edits keys and four-word rules, and there is no field a
 * command line could arrive in.
 */
import { Plus, RotateCcw, Trash2 } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { POLICY_ACTION_LABEL_KEYS, POLICY_ACTIONS, POLICY_LEVEL_LABEL_KEYS, POLICY_LEVELS } from "@/lib/policy/labels";
import { copyPolicyRule, policyRecord, policyRows, samePolicyMatrix } from "@/lib/policy/matrix";
import { usePreferencesStore } from "@/stores/preferences";
import { DEFAULT_POLICY_MATRIX, type PolicyAction, type PolicyMatrix, type PolicyRule } from "@/types/app";
import type { RiskLevel } from "@/types/batch";

/** The three exception layers, in the order a decision looks at them. */
const SECTIONS = [
  {
    id: "groups",
    layer: "group",
    titleKey: "settings.security.policy.sections.groups.title",
    hintKey: "settings.security.policy.sections.groups.hint",
    placeholderKey: "settings.security.policy.sections.groups.placeholder",
  },
  {
    id: "hosts",
    layer: "host",
    titleKey: "settings.security.policy.sections.hosts.title",
    hintKey: "settings.security.policy.sections.hosts.hint",
    placeholderKey: "settings.security.policy.sections.hosts.placeholder",
  },
  {
    id: "directories",
    layer: "directory",
    titleKey: "settings.security.policy.sections.directories.title",
    hintKey: "settings.security.policy.sections.directories.hint",
    placeholderKey: "settings.security.policy.sections.directories.placeholder",
  },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

/**
 * A rule being edited.
 *
 * The wire type's fields are read-only because nothing outside the draft should write one. The
 * draft is the one place that does, so it removes the modifier here rather than copying the object
 * on every keystroke.
 */
type EditableRule = { -readonly [Level in keyof PolicyRule]: PolicyAction };

/** A row of an exception section: stable for the DOM while its key is being typed. */
interface DraftRow {
  /** Assigned once so a row keeps its identity while the person edits the key beside it. */
  readonly id: number;
  key: string;
  rule: EditableRule;
}

const { t } = useI18n();
const preferences = usePreferencesStore();

let nextRowId = 1;

function editableRule(rule: PolicyRule): EditableRule {
  return copyPolicyRule(rule);
}

function rowsOf(record: Readonly<Record<string, PolicyRule>>): DraftRow[] {
  return policyRows(record).map((row) => ({ id: nextRowId++, key: row.key, rule: editableRule(row.rule) }));
}

const globalRule = ref<EditableRule>(editableRule(DEFAULT_POLICY_MATRIX.global));
const rows = ref<Record<SectionId, DraftRow[]>>({ groups: [], hosts: [], directories: [] });

/** Puts the draft down and picks up a matrix — the store's, or the default after a restore. */
function loadDraft(matrix: PolicyMatrix): void {
  globalRule.value = editableRule(matrix.global);
  rows.value = {
    groups: rowsOf(matrix.groups),
    hosts: rowsOf(matrix.hosts),
    directories: rowsOf(matrix.directories),
  };
}

loadDraft(preferences.policyMatrix);

// A reload, a discard or another page's Apply replaces the store's matrix; the draft follows it.
// Watch, rather than read once: `discard` is a re-read of the file by design, and a draft left
// standing over it would be a page arguing with the settings that were just restored.
watch(
  () => preferences.policyMatrix,
  (matrix) => loadDraft(matrix),
);

const actionOptions = computed(() =>
  POLICY_ACTIONS.map((value) => ({ value, label: t(POLICY_ACTION_LABEL_KEYS[value]) })),
);

/** The matrix the draft describes: named rows become entries, unnamed ones stay on screen. */
function buildMatrix(): PolicyMatrix {
  return {
    global: copyPolicyRule(globalRule.value),
    groups: policyRecord(rows.value.groups),
    hosts: policyRecord(rows.value.hosts),
    directories: policyRecord(rows.value.directories),
  };
}

const changed = computed(() => !samePolicyMatrix(buildMatrix(), preferences.policyMatrix));

const notice = ref<"" | "applied" | "failed">("");

/** "Applied" is about the draft it was pressed on; the next edit is a different draft. */
function noteEdited(): void {
  notice.value = "";
}

function setRule(rule: EditableRule, level: RiskLevel, action: string): void {
  noteEdited();
  rule[level] = action as PolicyAction;
}

function addRow(section: SectionId): void {
  noteEdited();
  // A new row starts at the built-in rule, not at the global one on screen: the global rule may
  // already be tightened, and a row that appeared pre-tightened would suggest a decision nobody
  // made. Editing it is how it becomes an exception.
  rows.value[section].push({
    id: nextRowId++,
    key: "",
    rule: editableRule(DEFAULT_POLICY_MATRIX.global),
  });
}

function removeRow(section: SectionId, id: number): void {
  noteEdited();
  rows.value[section] = rows.value[section].filter((row) => row.id !== id);
}

const failure = computed(() =>
  notice.value === "failed" && preferences.problem ? describeShellError(translate, preferences.problem) : null,
);

/** Applies the draft to the store and writes the settings file — the page's only write. */
async function apply(): Promise<void> {
  notice.value = "";
  preferences.setPolicyMatrix(buildMatrix());
  notice.value = (await preferences.commit()) ? "applied" : "failed";
}

/** Puts the draft back to the built-in default. Nothing is written until Apply. */
function restoreDefaults(): void {
  notice.value = "";
  loadDraft(DEFAULT_POLICY_MATRIX);
  // The draft may now equal the store's matrix; the Apply button reads that for itself.
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** The label a select is given for a reader who cannot see the row it is in. */
function ruleLabel(level: RiskLevel): string {
  return t(POLICY_LEVEL_LABEL_KEYS[level]);
}
</script>

<template>
  <section
    data-policy-matrix
    class="space-y-3 rounded-md border border-border bg-muted/20 px-3 py-3"
    :aria-label="t('settings.security.policy.title')"
  >
    <div class="min-w-0">
      <h4 class="text-sm font-medium">{{ t("settings.security.policy.title") }}</h4>
      <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
        {{ t("settings.security.policy.description") }}
      </p>
    </div>

    <!-- The inheritance rule, said once at the top rather than repeated beside every row. -->
    <p data-policy-most-restrictive class="text-[11px] leading-relaxed text-muted-foreground">
      {{ t("settings.security.policy.mostRestrictive") }}
    </p>

    <!-- The global rule: what every action is, before any exception is looked at. -->
    <div data-policy-layer="global" class="space-y-2 rounded-md border border-border/70 bg-background/40 px-3 py-2.5">
      <h5 class="text-xs font-medium">{{ t("settings.security.policy.global.title") }}</h5>
      <div class="grid gap-2 sm:grid-cols-2 @4xl:grid-cols-4">
        <div v-for="level in POLICY_LEVELS" :key="level" class="space-y-1">
          <label :for="`policy-global-${level}`" class="block text-[11px] font-medium text-muted-foreground">
            {{ t(POLICY_LEVEL_LABEL_KEYS[level]) }}
          </label>
          <Select
            :id="`policy-global-${level}`"
            :model-value="globalRule[level]"
            :options="actionOptions"
            size="compact"
            trigger-class="w-full"
            :aria-label="ruleLabel(level)"
            :data-policy-rule="level"
            @update:model-value="(value: string) => setRule(globalRule, level, value)"
          />
        </div>
      </div>
      <!--
        The resolver's own rule, stated where the two levels it applies to are chosen. It stays
        visible whichever action is selected: a reader looking at "allow" needs it most, and a
        reader looking at "deny" is not harmed by knowing the floor.
      -->
      <p data-policy-normalized class="text-[11px] leading-relaxed text-muted-foreground">
        {{ t("settings.security.policy.normalized") }}
      </p>
    </div>

    <!-- The three exception layers, each a list of keyed rows. -->
    <div
      v-for="section in SECTIONS"
      :key="section.id"
      :data-policy-layer="section.layer"
      class="space-y-2 border-t border-border/60 pt-3"
    >
      <div class="min-w-0">
        <h5 class="text-xs font-medium">{{ t(section.titleKey) }}</h5>
        <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t(section.hintKey) }}</p>
      </div>

      <p v-if="rows[section.id].length === 0" class="text-[11px] text-muted-foreground">
        {{ t("settings.security.policy.sections.empty") }}
      </p>

      <div
        v-for="row in rows[section.id]"
        :key="row.id"
        class="flex flex-wrap items-center gap-2"
        :data-policy-row="row.id"
      >
        <label class="sr-only" :for="`policy-key-${section.id}-${row.id}`">
          {{ t("settings.security.policy.key") }}
        </label>
        <input
          :id="`policy-key-${section.id}-${row.id}`"
          v-model="row.key"
          type="text"
          data-policy-key
          spellcheck="false"
          autocomplete="off"
          :placeholder="t(section.placeholderKey)"
          class="min-w-40 flex-1 rounded-md border border-border bg-background px-2.5 py-1.5 font-mono text-[11px] transition-colors focus-visible:border-primary focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
          @input="noteEdited()"
        />
        <Select
          v-for="level in POLICY_LEVELS"
          :key="level"
          :model-value="row.rule[level]"
          :options="actionOptions"
          size="compact"
          trigger-class="w-24"
          :aria-label="ruleLabel(level)"
          :data-policy-rule="level"
          @update:model-value="(value: string) => setRule(row.rule, level, value)"
        />
        <button
          type="button"
          data-policy-remove
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :aria-label="t('settings.security.policy.remove')"
          :title="t('settings.security.policy.remove')"
          @click="removeRow(section.id, row.id)"
        >
          <Trash2 :size="12" />
        </button>
      </div>

      <button
        type="button"
        :data-policy-add="section.layer"
        class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        @click="addRow(section.id)"
      >
        <Plus :size="12" />
        {{ t("settings.security.policy.add") }}
      </button>
    </div>

    <div class="flex flex-wrap items-center gap-2 border-t border-border/60 pt-3">
      <button
        type="button"
        data-policy-restore
        class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        @click="restoreDefaults()"
      >
        <RotateCcw :size="12" />
        {{ t("settings.security.policy.restore") }}
      </button>
      <span class="flex-1" />
      <span v-if="notice === 'applied'" data-policy-notice class="text-[11px] text-muted-foreground">
        {{ t("settings.security.policy.applied") }}
      </span>
      <button
        type="button"
        data-policy-apply
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
        :disabled="!changed"
        @click="apply()"
      >
        {{ t("settings.security.policy.apply") }}
      </button>
    </div>

    <p
      v-if="failure"
      data-policy-problem
      role="alert"
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
    >
      <span>{{ failure.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ failure.code }}</span>
    </p>
  </section>
</template>
