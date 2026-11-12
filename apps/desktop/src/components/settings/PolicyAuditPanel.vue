<script setup lang="ts">
/**
 * The policy trail, read-only: what the matrix decided about the last unattended actions.
 *
 * Three rules:
 *
 * 1. **The file is the record.** The panel reads the tail of the JSONL trail when it opens and when
 *    somebody presses Refresh, rather than collecting what the window happened to see — an agent's
 *    call made while this page was closed has to appear, and a page that accumulated events would
 *    disagree with the file as soon as anything ran.
 * 2. **No command contents.** An event carries the operation's name, the classifier's verdict and
 *    the layer that decided; the subject is the redacted line the host chose to keep, and nothing
 *    here asks for more.
 * 3. **Newest last.** The host answers in the order the file was written, which is the order the
 *    table draws: the most recent decision is the bottom row, where a reader arriving from another
 *    page expects it.
 *
 * The rows carry the session, group, host and remote directory the decision looked at — the same
 * fields no audit line may hold more of. A local path has no column here and no field to arrive in.
 */
import { RefreshCw } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { POLICY_ACTION_LABEL_KEYS, POLICY_LEVEL_LABEL_KEYS } from "@/lib/policy/labels";
import { TABLE, rowClass } from "@/lib/table";
import type { PolicyAction, PolicyAuditEvent } from "@/types/app";

const props = withDefaults(
  defineProps<{
    /** How many of the newest decisions to read. */
    limit?: number;
  }>(),
  { limit: 100 },
);

const { t } = useI18n();

const events = ref<PolicyAuditEvent[]>([]);
const loading = ref(false);
const failure = ref<ShellError | null>(null);

const problem = computed(() => (failure.value ? describeShellError(translate, failure.value) : null));

/** Reads the tail again. The first read happens when the panel is drawn. */
async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    events.value = await backend.policy.auditTail(props.limit);
  } catch (error) {
    failure.value = asShellError(error, "policy_audit_tail");
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void load();
});

/** The trail writes seconds, the same clock the rest of the settings files use. */
function when(at: number): string {
  return new Date(at * 1000).toLocaleString();
}

function decisionClass(decision: PolicyAction): string {
  if (decision === "allow") {
    return "text-success";
  }
  return decision === "deny" ? "text-destructive" : "text-muted-foreground";
}

/**
 * The session, group, host and directory a decision looked at, as one cell.
 *
 * Labelled rather than juxtaposed: `web` and `web:22` are a session and a host, and a table that
 * drew them both unadorned would leave a reader guessing which is which.
 */
function targets(entry: PolicyAuditEvent): string[] {
  const parts: string[] = [];
  if (entry.sessionId) {
    parts.push(`${t("settings.security.policy.audit.fields.session")}: ${entry.sessionId}`);
  }
  if (entry.groupId) {
    parts.push(`${t("settings.security.policy.audit.fields.group")}: ${entry.groupId}`);
  }
  if (entry.host) {
    parts.push(`${t("settings.security.policy.audit.fields.host")}: ${entry.host}`);
  }
  if (entry.directory) {
    parts.push(`${t("settings.security.policy.audit.fields.directory")}: ${entry.directory}`);
  }
  return parts;
}

/** The rules behind the risk level, for the level's own tooltip. */
function riskReasons(entry: PolicyAuditEvent): string {
  return entry.risk.reasons.map((reason) => t(reason.labelKey)).join(" · ");
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <section
    data-policy-audit
    class="space-y-2 rounded-md border border-border bg-muted/20 px-3 py-3"
    :aria-label="t('settings.security.policy.audit.title')"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h4 class="text-sm font-medium">{{ t("settings.security.policy.audit.title") }}</h4>
        <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          {{ t("settings.security.policy.audit.description") }}
        </p>
      </div>
      <button
        type="button"
        data-policy-audit-refresh
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
        :disabled="loading"
        @click="load()"
      >
        <RefreshCw :size="12" :class="loading ? 'animate-spin' : undefined" />
        {{ t("settings.security.policy.audit.refresh") }}
      </button>
    </div>

    <p
      v-if="problem"
      data-policy-audit-problem
      role="alert"
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
    >
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <div class="max-h-64 overflow-y-auto select-text">
      <table :class="TABLE.page.table" :aria-label="t('settings.security.policy.audit.title')">
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.at") }}
            </th>
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.action") }}
            </th>
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.risk") }}
            </th>
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.decision") }}
            </th>
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.source") }}
            </th>
            <th class="text-left" :class="TABLE.page.headCell">
              {{ t("settings.security.policy.audit.columns.target") }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="(entry, index) in events"
            :key="`${entry.at}-${index}`"
            :data-policy-audit-row="index"
            :class="rowClass('page')"
          >
            <td class="whitespace-nowrap font-mono text-[11px]" :class="TABLE.page.cell">
              {{ when(entry.at) }}
            </td>
            <td class="font-mono text-[11px]" :class="TABLE.page.cell">
              <span class="block truncate" :title="entry.subject ?? entry.action">{{ entry.action }}</span>
              <span
                v-if="entry.subject"
                class="block truncate text-[11px] text-muted-foreground"
                :title="entry.subject"
              >
                {{ entry.subject }}
              </span>
            </td>
            <td :class="[TABLE.page.cell, 'text-muted-foreground']" :title="riskReasons(entry)">
              {{ t(POLICY_LEVEL_LABEL_KEYS[entry.risk.level]) }}
            </td>
            <td :class="[TABLE.page.cell, decisionClass(entry.decision)]">
              {{ t(POLICY_ACTION_LABEL_KEYS[entry.decision]) }}
            </td>
            <td class="font-mono text-[11px]" :class="TABLE.page.cell">{{ entry.source }}</td>
            <td :class="[TABLE.page.cell, 'font-mono text-[11px] text-muted-foreground']">
              <span v-if="targets(entry).length > 0">{{ targets(entry).join(" · ") }}</span>
              <span v-else>{{ t("settings.security.policy.audit.none") }}</span>
            </td>
          </tr>
          <tr v-if="events.length === 0">
            <td colspan="6" :class="TABLE.page.emptyCell">
              {{
                loading
                  ? t("settings.security.policy.audit.loading")
                  : problem
                    ? t("settings.security.policy.audit.failed")
                    : t("settings.security.policy.audit.empty")
              }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
