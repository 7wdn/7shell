<script setup lang="ts">
/**
 * The agent interface: one switch, the four numbers behind it, and what it has been doing.
 *
 * Four rules shape this block, and each of them comes from something the page would otherwise say
 * wrongly:
 *
 * 1. **The listener is the authority, not the settings file.** "On" is what the status says, and the
 *    port shown is the one it answered with — a settings file that says 0 while the listener is on
 *    51234 must not draw "port 0".
 * 2. **The master switch is a command.** Turning it on starts a socket; a checkbox that only wrote a
 *    file would claim something no process is doing. And turning it on saves first, so the listener
 *    starts with the port and tier that are on screen rather than the ones last written.
 * 3. **The client configuration is copied, never drawn.** It names a program and a file, which is
 *    what a client's configuration is for — and the token it does *not* carry stays in the endpoint
 *    file, where this window cannot reach it.
 * 4. **The audit is read from the file, not collected in memory.** A page that accumulated what it
 *    happened to see would disagree with the log as soon as anything happened while it was closed.
 */
import { Check, Copy, LoaderCircle, RefreshCw } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import SettingsHelp from "@/components/settings/SettingsHelp.vue";
import { describeShellError } from "@/i18n/errors";
import {
  MCP_AUDIT_STATUS_LABEL_KEYS,
  MCP_SCOPE_HINT_KEYS,
  MCP_SCOPE_LABEL_KEYS,
  RISK_LEVEL_LABEL_KEYS,
} from "@/lib/mcp/labels";
import { TABLE, rowClass } from "@/lib/table";
import { useMcpStore } from "@/stores/mcp";
import { usePreferencesStore } from "@/stores/preferences";
import {
  MCP_APPROVAL_TIMEOUT_SECONDS_MAX,
  MCP_APPROVAL_TIMEOUT_SECONDS_MIN,
  MCP_AUDIT_RETENTION_DAYS_MAX,
  MCP_AUDIT_RETENTION_DAYS_MIN,
  MCP_MAX_RESULT_BYTES_MAX,
  MCP_MAX_RESULT_BYTES_MIN,
  MCP_PORT_MAX,
  isMcpScope,
} from "@/types/mcp";
import { DEFAULT_MCP_SCOPE, type McpScope } from "@/types/app";
import type { McpAuditEntry } from "@/types/mcp";

const { t } = useI18n();
const mcp = useMcpStore();
const preferences = usePreferencesStore();

/** What the last save did, said under the button. */
const notice = ref<"" | "saved" | "savedRestarted" | "restartRequired">("");
/** What the last copy did, said under its button. */
const copyNotice = ref<"" | "copied" | "failed">("");

const problem = computed(() => (mcp.failure ? describeShellError(translate, mcp.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const scope = computed<McpScope>(() => preferences.mcpScope ?? DEFAULT_MCP_SCOPE);

/** The tier the listener is actually running with, which is not always the one being chosen. */
const runningScope = computed<McpScope>(() => {
  const reported = mcp.status?.scope;
  return isMcpScope(reported) ? reported : scope.value;
});

/** The three tiers, with the line under the control saying what the chosen one allows. */
const scopeOptions = computed(() =>
  (Object.keys(MCP_SCOPE_LABEL_KEYS) as McpScope[]).map((value) => ({
    value,
    label: t(MCP_SCOPE_LABEL_KEYS[value]),
  })),
);
const scopeHint = computed(() => t(MCP_SCOPE_HINT_KEYS[scope.value]));

/** The result limit, in the unit the field says rather than in bytes. */
const resultKiB = computed(() => Math.round(preferences.mcpMaxResultBytes / 1024));

/** The port the listener answered with, when it is on: zero in the settings means "let the system". */
const shownPort = computed(() => {
  const running = mcp.status?.enabled === true ? mcp.status.port : null;
  if (preferences.mcpPort !== 0) {
    return String(preferences.mcpPort);
  }
  return t("mcp.status.portAutomatic", { port: running === null ? "—" : String(running) });
});

/** A typed number, with an empty field reading as nought rather than as "not a number". */
function typedNumber(event: Event): number {
  return Number((event.target as HTMLInputElement).value || "0");
}

/**
 * Turns the listener on or off.
 *
 * On saves first: the listener reads the settings file when it starts, so starting it with an
 * unsaved port on screen would start it with the old one. Off needs no save — there is nothing to
 * read on the way down.
 */
async function toggleAccess(on: boolean): Promise<void> {
  notice.value = "";
  if (on && !(await preferences.commit())) {
    return;
  }
  const changed = on ? await mcp.enable() : await mcp.disable();
  if (changed && mcp.status) {
    preferences.adoptMcpEnabled(mcp.status.enabled);
  }
}

/** Writes the four numbers, then restarts a running listener so it is running what was written. */
async function save(): Promise<void> {
  notice.value = "";
  if (!(await preferences.commit())) {
    return;
  }
  if (mcp.status?.enabled !== true) {
    notice.value = "saved";
    return;
  }
  if (await mcp.enable()) {
    preferences.adoptMcpEnabled(true);
    notice.value = "savedRestarted";
    return;
  }
  notice.value = "restartRequired";
}

async function copyConfig(): Promise<void> {
  copyNotice.value = "";
  copyNotice.value = (await mcp.copyClientConfig()) ? "copied" : "failed";
}

function auditStatusClass(entry: McpAuditEntry): string {
  if (entry.status === "succeeded") {
    return "text-success";
  }
  return entry.status === "failed" ? "text-destructive" : "text-muted-foreground";
}

function when(at: number): string {
  // The audit writes seconds, the same clock the rest of the settings files use; `Date` reads
  // milliseconds, and passing seconds straight through would draw every call in 1970.
  return new Date(at * 1000).toLocaleString();
}

onMounted(() => {
  void mcp.refresh();
  void mcp.loadAudit();
});
</script>

<template>
  <section class="space-y-3">
    <p
      v-if="problem"
      data-mcp-problem
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
    >
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <!-- What the listener is doing, and the switch that decides it. -->
    <div class="space-y-3 rounded-md border border-border px-3 py-3">
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <div class="flex items-center gap-1.5">
            <h4 class="text-sm font-medium">{{ t("mcp.status.heading") }}</h4>
            <SettingsHelp :title="t('mcp.switch.label')" :help="t('mcp.switch.help')" />
          </div>
          <p class="mt-0.5 text-xs text-muted-foreground">{{ t("mcp.title") }}</p>
        </div>
        <Switch
          :model-value="mcp.status?.enabled === true"
          :disabled="mcp.changing || mcp.status === null"
          :aria-label="t('mcp.switch.label')"
          @update:model-value="toggleAccess"
        />
      </div>

      <!--
        The readings the listener answered with. Selectable, like every other reading on a page that
        is otherwise chrome: a port and a path are pasted into another program.
      -->
      <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs select-text">
        <dt class="text-muted-foreground">{{ t("mcp.status.heading") }}</dt>
        <dd class="min-w-0">
          <span v-if="mcp.status === null" class="text-muted-foreground">{{ t("mcp.status.unknown") }}</span>
          <span v-else-if="mcp.status.enabled" class="text-success">
            {{ t("mcp.status.running", { port: String(mcp.status.port ?? "—") }) }}
          </span>
          <span v-else class="text-muted-foreground">{{ t("mcp.status.stopped") }}</span>
        </dd>

        <dt class="text-muted-foreground">{{ t("mcp.status.port") }}</dt>
        <dd class="font-mono text-[11px] break-all">{{ shownPort }}</dd>

        <dt class="text-muted-foreground">{{ t("mcp.status.scope") }}</dt>
        <dd>{{ t(MCP_SCOPE_LABEL_KEYS[runningScope]) }}</dd>

        <dt class="text-muted-foreground">{{ t("mcp.status.pending") }}</dt>
        <dd :class="(mcp.status?.pendingApprovals ?? 0) > 0 ? 'text-warning' : 'text-muted-foreground'">
          {{
            (mcp.status?.pendingApprovals ?? 0) > 0
              ? t("mcp.status.pendingCount", { count: String(mcp.status?.pendingApprovals ?? 0) })
              : t("mcp.status.pendingNone")
          }}
        </dd>

        <template v-if="mcp.status?.auditPath">
          <dt class="text-muted-foreground">{{ t("mcp.status.auditPath") }}</dt>
          <dd class="font-mono text-[11px] break-all">{{ mcp.status.auditPath }}</dd>
        </template>
      </dl>
      <p class="text-xs text-muted-foreground">{{ t("mcp.status.countNote") }}</p>
    </div>

    <!-- The snippet a client's configuration takes: copied, never drawn. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <div class="space-y-1">
        <h4 class="text-sm font-medium">{{ t("mcp.config.title") }}</h4>
        <p class="text-xs leading-relaxed text-muted-foreground">{{ t("mcp.config.description") }}</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="mcp.copying"
          @click="copyConfig()"
        >
          <LoaderCircle v-if="mcp.copying" :size="12" class="animate-spin" />
          <Check v-else-if="copyNotice === 'copied'" :size="12" class="text-success" />
          <Copy v-else :size="12" />
          {{ t("mcp.config.copy") }}
        </button>
        <span v-if="copyNotice === 'copied'" class="text-xs text-success">{{ t("mcp.config.copied") }}</span>
        <span v-else-if="copyNotice === 'failed'" class="text-xs text-destructive">
          {{ t("mcp.config.copyFailed") }}
        </span>
      </div>
    </div>

    <!-- The four fields, and the button that writes them. -->
    <div class="space-y-3 rounded-md border border-border px-3 py-3">
      <h4 class="text-sm font-medium">{{ t("mcp.fields.heading") }}</h4>

      <div class="grid gap-3 sm:grid-cols-2">
        <div>
          <label class="block text-xs" for="mcp-port">{{ t("mcp.fields.port") }}</label>
          <input
            id="mcp-port"
            type="number"
            min="0"
            :max="MCP_PORT_MAX"
            step="1"
            class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
            :value="preferences.mcpPort"
            @input="preferences.setMcpPort(typedNumber($event))"
          />
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.fields.portHint") }}</p>
        </div>

        <div>
          <label class="block text-xs" for="mcp-approval-timeout">{{ t("mcp.fields.approvalTimeout") }}</label>
          <div class="mt-1 flex items-center gap-2">
            <input
              id="mcp-approval-timeout"
              type="number"
              :min="MCP_APPROVAL_TIMEOUT_SECONDS_MIN"
              :max="MCP_APPROVAL_TIMEOUT_SECONDS_MAX"
              step="1"
              class="w-24 rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
              :value="preferences.mcpApprovalTimeoutSeconds"
              @input="preferences.setMcpApprovalTimeoutSeconds(typedNumber($event))"
            />
            <span class="text-xs text-muted-foreground">{{ t("mcp.fields.unitSeconds") }}</span>
          </div>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
            {{ t("mcp.fields.approvalTimeoutHint") }}
          </p>
        </div>

        <div>
          <label class="block text-xs" for="mcp-retention">{{ t("mcp.fields.retentionDays") }}</label>
          <div class="mt-1 flex items-center gap-2">
            <input
              id="mcp-retention"
              type="number"
              :min="MCP_AUDIT_RETENTION_DAYS_MIN"
              :max="MCP_AUDIT_RETENTION_DAYS_MAX"
              step="1"
              class="w-24 rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
              :value="preferences.mcpAuditRetentionDays"
              @input="preferences.setMcpAuditRetentionDays(typedNumber($event))"
            />
            <span class="text-xs text-muted-foreground">{{ t("mcp.fields.unitDays") }}</span>
          </div>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.fields.retentionDaysHint") }}</p>
        </div>

        <div>
          <label class="block text-xs" for="mcp-max-result">{{ t("mcp.fields.maxResult") }}</label>
          <div class="mt-1 flex items-center gap-2">
            <input
              id="mcp-max-result"
              type="number"
              :min="MCP_MAX_RESULT_BYTES_MIN / 1024"
              :max="MCP_MAX_RESULT_BYTES_MAX / 1024"
              step="1"
              class="w-24 rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
              :value="resultKiB"
              @input="preferences.setMcpMaxResultBytes(typedNumber($event) * 1024)"
            />
            <span class="text-xs text-muted-foreground">{{ t("mcp.fields.unitKiB") }}</span>
          </div>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.fields.maxResultHint") }}</p>
        </div>

        <div class="sm:col-span-2">
          <label class="block text-xs" for="mcp-scope">{{ t("mcp.fields.scope") }}</label>
          <div class="mt-1">
            <Select
              id="mcp-scope"
              :model-value="scope"
              :options="scopeOptions"
              :aria-label="t('mcp.fields.scope')"
              @update:model-value="preferences.setMcpScope($event as McpScope)"
            />
          </div>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ scopeHint }}</p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
          :disabled="mcp.changing"
          @click="save()"
        >
          {{ t("mcp.fields.save") }}
        </button>
        <span v-if="notice === 'saved'" class="text-xs text-muted-foreground">{{ t("mcp.fields.saved") }}</span>
        <span v-else-if="notice === 'savedRestarted'" class="text-xs text-success">
          {{ t("mcp.fields.savedRestarted") }}
        </span>
        <span v-else-if="notice === 'restartRequired'" class="text-xs text-warning">
          {{ t("mcp.fields.restartRequired") }}
        </span>
      </div>
    </div>

    <!-- The tail of the audit file: what was called, how it ended, how long it took. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h4 class="text-sm font-medium">{{ t("mcp.audit.title") }}</h4>
          <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.audit.description") }}</p>
        </div>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
          :disabled="mcp.loadingAudit"
          @click="mcp.loadAudit()"
        >
          <RefreshCw :size="12" :class="mcp.loadingAudit ? 'animate-spin' : undefined" />
          {{ t("mcp.audit.refresh") }}
        </button>
      </div>

      <div class="max-h-72 overflow-y-auto select-text">
        <table :class="TABLE.page.table" :aria-label="t('mcp.audit.title')">
          <thead :class="TABLE.page.head">
            <tr :class="TABLE.page.headRow">
              <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.audit.columns.at") }}</th>
              <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.audit.columns.tool") }}</th>
              <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.audit.columns.status") }}</th>
              <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.audit.columns.risk") }}</th>
              <th class="text-right" :class="TABLE.page.headCell">{{ t("mcp.audit.columns.duration") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(entry, index) in mcp.audit" :key="`${entry.at}-${index}`" :class="rowClass('page')">
              <td class="whitespace-nowrap font-mono text-[11px]" :class="TABLE.page.cell">{{ when(entry.at) }}</td>
              <td class="font-mono text-[11px]" :class="TABLE.page.cell">{{ entry.tool }}</td>
              <td :class="[TABLE.page.cell, auditStatusClass(entry)]">
                {{ t(MCP_AUDIT_STATUS_LABEL_KEYS[entry.status]) }}
              </td>
              <td :class="[TABLE.page.cell, 'text-muted-foreground']">
                {{ entry.risk ? t(RISK_LEVEL_LABEL_KEYS[entry.risk.level]) : t("mcp.audit.none") }}
              </td>
              <td class="text-right font-mono text-[11px]" :class="TABLE.page.cell">
                {{ `${entry.durationMs} ms` }}
              </td>
            </tr>
            <tr v-if="mcp.audit.length === 0">
              <td colspan="5" :class="TABLE.page.emptyCell">
                {{ mcp.loadingAudit ? t("mcp.audit.loading") : t("mcp.audit.empty") }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>
