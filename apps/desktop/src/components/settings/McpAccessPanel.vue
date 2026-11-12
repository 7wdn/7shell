<script setup lang="ts">
/**
 * The access tab: whether anything is listening, how a client starts the bridge, the four numbers
 * behind it, the external servers it calls, and what it has been doing.
 *
 * Five rules shape this block, and each of them comes from something the page would otherwise say
 * wrongly:
 *
 * 1. **The listener is the authority, not the settings file.** "On" is what the status says, and the
 *    port shown is the one it answered with — a settings file that says 0 while the listener is on
 *    51234 must not draw "port 0".
 * 2. **The master switch is a command.** Turning it on starts a socket; a checkbox that only wrote a
 *    file would claim something no process is doing. And turning it on saves first, so the listener
 *    starts with the port and tier that are on screen rather than the ones last written.
 * 3. **A client template says where the bridge is, never what it carries.** The snippets are
 *    rebuilt from the host's own answer, keeping only the endpoint variable; the token never leaves
 *    the endpoint file, and a future field in that answer cannot reach a clipboard from here.
 * 4. **The audit is read from the file, not collected in memory.** A page that accumulated what it
 *    happened to see would disagree with the log as soon as anything happened while it was closed.
 * 5. **A server's row is the host's answer.** Its state, its tool count and whether secrets are
 *    held come from `mcp_servers_list` and the `mcp:server_status` event — never from the form.
 */
import { Check, ChevronRight, Copy, KeyRound, LoaderCircle, Pencil, Plus, RefreshCw, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import SettingsHelp from "@/components/settings/SettingsHelp.vue";
import {
  MCP_AUDIT_STATUS_LABEL_KEYS,
  MCP_CLIENT_LABEL_KEYS,
  MCP_SCOPE_HINT_KEYS,
  MCP_SCOPE_LABEL_KEYS,
  MCP_SERVER_STATE_LABEL_KEYS,
  MCP_SERVER_TRANSPORT_LABEL_KEYS,
  RISK_LEVEL_LABEL_KEYS,
} from "@/lib/mcp/labels";
import {
  MCP_CLIENTS,
  MCP_CLIENT_LOCATIONS,
  clientConfigSnippet,
  parseBridgeConfig,
  type McpClientId,
} from "@/lib/mcp/clients";
import { type KeyValueDraft, keyValueRecord, parseServerArgs } from "@/lib/mcp/servers";
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
  MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT,
  MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX,
  MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN,
  MCP_SERVER_TRANSPORTS,
  isMcpScope,
  isValidMcpServerId,
  type McpAuditEntry,
  type McpServerConfig,
  type McpServerState,
  type McpServerTransport,
  type McpServerView,
} from "@/types/mcp";
import { DEFAULT_MCP_SCOPE, type McpScope } from "@/types/app";

const { t } = useI18n();
const mcp = useMcpStore();
const preferences = usePreferencesStore();

/** What the last save did, said under the button. */
const notice = ref<"" | "saved" | "savedRestarted" | "restartRequired">("");
/** What the last copy did, said under its button. */
const copyNotice = ref<"" | "copied" | "failed">("");
/** Whether the audit tail is on screen; it is read from the file when it is first opened. */
const auditOpen = ref(false);

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

// -- Client templates ---------------------------------------------------------
//
// The host builds one `mcpServers` snippet naming the bridge beside this application. These tabs are
// the same two facts in the shape each client reads; nothing is fetched from the bridge here, and
// nothing but the endpoint variable travels — see `lib/mcp/clients.ts` for the rule.

/** Which client's template is on screen. One at a time, like the configuration itself. */
const client = ref<McpClientId>("claudeCode");
/** Whether a copy is in flight, so the button does not send the same text twice. */
const copyBusy = ref(false);

/** The two facts the host's answer holds, or nothing when it names no program. */
const bridge = computed(() => parseBridgeConfig(mcp.status?.clientConfig ?? ""));

/** The snippet on screen, in the chosen client's shape. */
const clientSnippet = computed(() => clientConfigSnippet(client.value, bridge.value, mcp.status?.clientConfig ?? ""));

/** The line under the snippet: where this client keeps its MCP servers. */
const clientLocation = computed(() => MCP_CLIENT_LOCATIONS[client.value]);

/** The client's name for the tab strip and the copy button's own sentence. */
function clientName(id: McpClientId): string {
  return t(MCP_CLIENT_LABEL_KEYS[id]);
}

function chooseClient(next: McpClientId): void {
  client.value = next;
  copyNotice.value = "";
}

/**
 * Puts the template on the clipboard.
 *
 * The same text that is drawn, and only that: it is two facts about where the bridge is, which is
 * exactly what somebody pastes into another program. The token is not one of them.
 */
async function copySnippet(): Promise<void> {
  copyNotice.value = "";
  const text = clientSnippet.value;
  if (text === "") {
    return;
  }
  copyBusy.value = true;
  try {
    await navigator.clipboard.writeText(text);
    copyNotice.value = "copied";
  } catch {
    copyNotice.value = "failed";
  } finally {
    copyBusy.value = false;
  }
}

function auditStatusClass(entry: McpAuditEntry): string {
  if (entry.status === "succeeded") {
    return "text-success";
  }
  return entry.status === "failed" ? "text-destructive" : "text-muted-foreground";
}

/**
 * Shows or hides the audit tail, reading it the first time it is shown.
 *
 * The file is the record, so opening the panel is a read rather than a redraw of what was there
 * when the page last looked — anything an agent did while the panel was closed has to appear.
 */
function toggleAudit(): void {
  auditOpen.value = !auditOpen.value;
  if (auditOpen.value) {
    void mcp.loadAudit();
  }
}

function when(at: number): string {
  // The audit writes seconds, the same clock the rest of the settings files use; `Date` reads
  // milliseconds, and passing seconds straight through would draw every call in 1970.
  return new Date(at * 1000).toLocaleString();
}

// -- External servers ---------------------------------------------------------
//
// Everything below is about the servers this installation calls beside its own tools. Three rules:
//
// 1. **The row is the host's answer.** Its state, its tool count and whether secrets are held come
//    from `mcp_servers_list` and the `mcp:server_status` event — never from the form.
// 2. **Secrets go in and do not come back.** The editor is empty every time it opens, saving
//    replaces the whole set, and `hasSecrets` is the only statement the page makes about the vault.
//    Saving with no entries clears what is stored, which is the command's own contract.
// 3. **An id is part of every tool name.** It is chosen once and then fixed: renaming it would
//    orphan the secrets filed under it and break calls an agent already knows.

/** One server's configuration while it is being edited. Secrets are not part of it. */
interface ServerDraft {
  id: string;
  isNew: boolean;
  name: string;
  transport: McpServerTransport;
  command: string;
  args: string;
  url: string;
  toolTimeoutSeconds: number;
  enabled: boolean;
}

/** The server whose configuration form is open, if any. */
const serverDraft = ref<ServerDraft | null>(null);
/** What the last server write did, said under the list. */
const serverNotice = ref<"" | "saved" | "removed">("");
/** The server whose removal is being asked about. */
const removingServer = ref<McpServerView | null>(null);
/** The server whose secrets editor is open; one at a time, like the configuration form. */
const secretsFor = ref<string | null>(null);
/** The two maps being typed. They start empty and are emptied again after every save. */
const envRows = ref<KeyValueDraft[]>([blankRow()]);
const headerRows = ref<KeyValueDraft[]>([blankRow()]);
const secretsNotice = ref<"" | "saved" | "cleared">("");

function blankRow(): KeyValueDraft {
  return { key: "", value: "" };
}

/** The transports the form offers, in the order the dropdown lists them. */
const transportOptions = computed(() =>
  MCP_SERVER_TRANSPORTS.map((value) => ({ value, label: t(MCP_SERVER_TRANSPORT_LABEL_KEYS[value]) })),
);

/**
 * What is wrong with the draft, as the sentence to draw under the form.
 *
 * Checked as one answer rather than as four red boxes: the reason Save stays disabled is the first
 * thing that must be fixed, and four messages at once would be a form arguing with itself.
 */
const serverDraftProblem = computed<string | null>(() => {
  const current = serverDraft.value;
  if (!current) {
    return null;
  }
  const id = current.id.trim();
  if (!isValidMcpServerId(id)) {
    return t("mcp.servers.validation.id");
  }
  if (current.isNew && mcp.servers.some((view) => view.config.id === id)) {
    return t("mcp.servers.validation.idTaken");
  }
  if (current.transport === "stdio" && current.command.trim() === "") {
    return t("mcp.servers.validation.command");
  }
  if (current.transport !== "stdio" && current.url.trim() === "") {
    return t("mcp.servers.validation.url");
  }
  return null;
});

const canSaveServerDraft = computed(() => serverDraft.value !== null && serverDraftProblem.value === null);

/** The secrets rows as the command takes them, or nothing while a row cannot be one. */
const envRecord = computed(() => keyValueRecord(envRows.value));
const headerRecord = computed(() => keyValueRecord(headerRows.value));
const secretsProblem = computed<string | null>(() =>
  envRecord.value === null || headerRecord.value === null ? t("mcp.servers.secrets.invalid") : null,
);

function serverName(view: McpServerView): string {
  return view.config.name.trim() === "" ? view.config.id : view.config.name;
}

function serverStateClass(state: McpServerState): string {
  switch (state) {
    case "ready":
      return "text-success";
    case "connecting":
      return "text-warning";
    case "failed":
      return "text-destructive";
    default:
      return "text-muted-foreground";
  }
}

function serverDotClass(state: McpServerState): string {
  switch (state) {
    case "ready":
      return "bg-success";
    case "connecting":
      return "bg-warning animate-pulse";
    case "failed":
      return "bg-destructive";
    default:
      return "bg-muted-foreground";
  }
}

/**
 * Whether the vault holds secrets for a server, said as one of three answers.
 *
 * `null` is "cannot tell", not "none": a locked vault knows nothing, and drawing "no secrets saved"
 * over it would invite somebody to type a secret they had already saved.
 */
function secretsStateLabel(view: McpServerView): string {
  if (view.hasSecrets === true) {
    return t("mcp.servers.secrets.state.saved");
  }
  return t(view.hasSecrets === false ? "mcp.servers.secrets.state.missing" : "mcp.servers.secrets.state.locked");
}

function secretsStateClass(view: McpServerView): string {
  return view.hasSecrets === false ? "text-warning" : "text-muted-foreground";
}

function startNewServer(): void {
  serverNotice.value = "";
  serverDraft.value = {
    id: "",
    isNew: true,
    name: "",
    transport: "stdio",
    command: "",
    args: "",
    url: "",
    toolTimeoutSeconds: MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT,
    enabled: true,
  };
}

function startEditServer(view: McpServerView): void {
  serverNotice.value = "";
  serverDraft.value = {
    id: view.config.id,
    isNew: false,
    name: view.config.name,
    transport: view.config.transport,
    command: view.config.command ?? "",
    args: view.config.args.join("\n"),
    url: view.config.url ?? "",
    toolTimeoutSeconds: view.config.toolTimeoutSeconds,
    enabled: view.config.enabled,
  };
}

function cancelServerDraft(): void {
  serverDraft.value = null;
}

function chooseServerTransport(value: string): void {
  if (serverDraft.value && MCP_SERVER_TRANSPORTS.includes(value as McpServerTransport)) {
    serverDraft.value.transport = value as McpServerTransport;
  }
}

function setDraftEnabled(enabled: boolean): void {
  if (serverDraft.value) {
    serverDraft.value.enabled = enabled;
  }
}

/** A number typed into the timeout box, with an empty or unreadable box reading as the default. */
function typedServerTimeout(event: Event): void {
  const current = serverDraft.value;
  if (!current) {
    return;
  }
  const value = Number((event.target as HTMLInputElement).value || "0");
  current.toolTimeoutSeconds = Number.isFinite(value) ? Math.round(value) : MCP_SERVER_TOOL_TIMEOUT_SECONDS_DEFAULT;
}

function serverConfigFromDraft(current: ServerDraft): McpServerConfig {
  const stdio = current.transport === "stdio";
  return {
    id: current.id.trim(),
    name: current.name.trim(),
    transport: current.transport,
    command: stdio ? current.command.trim() : undefined,
    // Only what the chosen transport reads is sent: a URL left over from a previous choice must
    // not travel with a stdio server, and vice versa.
    args: stdio ? parseServerArgs(current.args) : [],
    url: stdio ? undefined : current.url.trim(),
    enabled: current.enabled,
    toolTimeoutSeconds: Math.min(
      Math.max(current.toolTimeoutSeconds, MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN),
      MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX,
    ),
  };
}

async function saveServerDraft(): Promise<void> {
  const current = serverDraft.value;
  if (!current || !canSaveServerDraft.value) {
    return;
  }
  serverNotice.value = "";
  if (await mcp.saveServer(serverConfigFromDraft(current))) {
    serverDraft.value = null;
    serverNotice.value = "saved";
  }
}

/** Turning the switch is a write of its own: a server that is off is a file change, not a form. */
async function toggleServer(view: McpServerView, enabled: boolean): Promise<void> {
  serverNotice.value = "";
  await mcp.saveServer({ ...view.config, enabled });
}

async function refreshServer(view: McpServerView): Promise<void> {
  serverNotice.value = "";
  await mcp.refreshServer(view.config.id);
}

async function confirmRemoveServer(): Promise<void> {
  const view = removingServer.value;
  removingServer.value = null;
  if (!view) {
    return;
  }
  serverNotice.value = "";
  if (await mcp.removeServer(view.config.id)) {
    serverNotice.value = "removed";
  }
}

function toggleSecrets(view: McpServerView): void {
  secretsNotice.value = "";
  if (secretsFor.value === view.config.id) {
    secretsFor.value = null;
    return;
  }
  secretsFor.value = view.config.id;
  // Never filled in from the vault: the values cannot be read back, so a box holding anything
  // would be this page inventing a secret. One empty row each is what an editor opens with.
  envRows.value = [blankRow()];
  headerRows.value = [blankRow()];
}

function addSecretRow(kind: "env" | "headers"): void {
  if (kind === "env") {
    envRows.value = [...envRows.value, blankRow()];
  } else {
    headerRows.value = [...headerRows.value, blankRow()];
  }
}

/** Removing the last row leaves an empty one, so the form never has nowhere to type. */
function removeSecretRow(kind: "env" | "headers", index: number): void {
  const rows = kind === "env" ? envRows.value : headerRows.value;
  const left = rows.filter((_, at) => at !== index);
  const next = left.length > 0 ? left : [blankRow()];
  if (kind === "env") {
    envRows.value = next;
  } else {
    headerRows.value = next;
  }
}

/**
 * Writes both maps, or clears them when nothing was typed.
 *
 * The rows are emptied afterwards whether they held secrets or not: what was typed has been sent,
 * and a box that kept it would be the one place the values could be read back from.
 */
async function saveSecrets(): Promise<void> {
  const serverId = secretsFor.value;
  const env = envRecord.value;
  const headers = headerRecord.value;
  if (serverId === null || env === null || headers === null) {
    return;
  }
  const empty = Object.keys(env).length === 0 && Object.keys(headers).length === 0;
  secretsNotice.value = "";
  if (await mcp.setServerSecrets(serverId, env, headers)) {
    envRows.value = [blankRow()];
    headerRows.value = [blankRow()];
    secretsNotice.value = empty ? "cleared" : "saved";
  }
}

onMounted(() => {
  void mcp.refresh();
  void mcp.loadServers();
});
</script>

<template>
  <section class="space-y-3">
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

    <!--
      The snippet each client's configuration takes: the bridge beside this application and the
      endpoint file, in the shape that client reads. Drawn rather than only copied, because the
      point of a template is seeing which file it goes into; the token is not part of either fact.
    -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3" data-mcp-clients>
      <div class="space-y-1">
        <h4 class="text-sm font-medium">{{ t("mcp.clients.title") }}</h4>
        <p class="text-xs leading-relaxed text-muted-foreground">{{ t("mcp.clients.description") }}</p>
      </div>

      <div class="flex flex-wrap gap-1 rounded-md border border-border bg-muted/30 p-1" role="tablist">
        <button
          v-for="entry in MCP_CLIENTS"
          :key="entry"
          type="button"
          role="tab"
          :aria-selected="client === entry"
          :data-mcp-client-tab="entry"
          class="rounded px-2.5 py-1 text-[11px] transition-colors"
          :class="
            client === entry ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'
          "
          @click="chooseClient(entry)"
        >
          {{ clientName(entry) }}
        </button>
      </div>

      <p class="text-[11px] text-muted-foreground">
        {{ t("mcp.clients.where") }}
        <span class="font-mono select-text">{{ clientLocation }}</span>
      </p>

      <p v-if="clientSnippet === ''" data-mcp-clients-unavailable class="text-xs leading-relaxed text-muted-foreground">
        {{ t("mcp.clients.unavailable") }}
      </p>
      <pre
        v-else
        data-mcp-client-snippet
        class="max-h-56 overflow-auto rounded-md border border-border bg-muted/20 px-3 py-2 font-mono text-[11px] leading-relaxed select-text"
        >{{ clientSnippet }}</pre
      >

      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-mcp-client-copy
          class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="copyBusy || clientSnippet === ''"
          @click="copySnippet()"
        >
          <LoaderCircle v-if="copyBusy" :size="12" class="animate-spin" />
          <Check v-else-if="copyNotice === 'copied'" :size="12" class="text-success" />
          <Copy v-else :size="12" />
          {{ t("mcp.clients.copy") }}
        </button>
        <span v-if="copyNotice === 'copied'" class="text-xs text-success">{{ t("mcp.clients.copied") }}</span>
        <span v-else-if="copyNotice === 'failed'" class="text-xs text-destructive">
          {{ t("mcp.clients.copyFailed") }}
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
          data-mcp-apply="access"
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

    <!--
      The external servers: one card each, the form that edits a configuration, and the editor for
      the secrets that configuration is reached with. The row is the host's answer, never the form.
    -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3" data-mcp-servers>
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h4 class="text-sm font-medium">{{ t("mcp.servers.title") }}</h4>
          <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.servers.description") }}</p>
        </div>
        <button
          type="button"
          data-mcp-server-add
          class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="serverDraft !== null || mcp.busyServer !== null"
          @click="startNewServer()"
        >
          <Plus :size="12" />
          {{ t("mcp.servers.add") }}
        </button>
      </div>

      <p v-if="mcp.servers.length === 0 && serverDraft === null" class="text-xs text-muted-foreground">
        {{ mcp.loadingServers ? t("mcp.servers.loading") : t("mcp.servers.empty") }}
      </p>

      <ul class="space-y-1.5">
        <li
          v-for="view in mcp.servers"
          :key="view.config.id"
          data-mcp-server-row
          :data-mcp-server-id="view.config.id"
          class="rounded-md border border-border bg-muted/20 px-3 py-2"
        >
          <div class="flex flex-wrap items-center gap-x-2 gap-y-0.5">
            <span
              class="size-1.5 shrink-0 rounded-full"
              :class="serverDotClass(view.status.state)"
              aria-hidden="true"
            ></span>
            <span class="text-xs font-medium">{{ serverName(view) }}</span>
            <span v-if="view.config.name.trim() !== ''" class="font-mono text-[11px] text-muted-foreground">
              {{ view.config.id }}
            </span>
            <span class="text-[11px] text-muted-foreground">
              {{ t(MCP_SERVER_TRANSPORT_LABEL_KEYS[view.config.transport]) }}
            </span>
            <span class="text-[11px]" :class="serverStateClass(view.status.state)">
              {{ t(MCP_SERVER_STATE_LABEL_KEYS[view.status.state]) }}
            </span>
            <span class="text-[11px] text-muted-foreground">
              {{ t("mcp.servers.toolCount", { count: String(view.status.tools) }) }}
            </span>
            <Switch
              class="ml-auto"
              :model-value="view.config.enabled"
              :disabled="mcp.busyServer !== null"
              :aria-label="t('mcp.servers.enabled')"
              @update:model-value="(value: boolean) => toggleServer(view, value)"
            />
          </div>

          <!-- A failure is the one thing on the row somebody has to act on, so it names its code. -->
          <p
            v-if="view.status.state === 'failed'"
            class="mt-0.5 flex flex-wrap items-baseline gap-x-2 text-[11px] text-destructive"
          >
            <span class="font-mono">{{ view.status.code }}</span>
            <span v-if="view.status.message">{{ view.status.message }}</span>
          </p>

          <div class="mt-1 flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              data-mcp-server-refresh
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="mcp.busyServer !== null"
              @click="refreshServer(view)"
            >
              <LoaderCircle v-if="mcp.busyServer === view.config.id" :size="10" class="animate-spin" />
              <RefreshCw v-else :size="10" />
              {{ t("mcp.servers.refresh") }}
            </button>
            <button
              type="button"
              data-mcp-server-edit
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="serverDraft !== null || mcp.busyServer !== null"
              @click="startEditServer(view)"
            >
              <Pencil :size="10" />
              {{ t("mcp.servers.edit") }}
            </button>
            <button
              type="button"
              data-mcp-server-secrets
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="mcp.busyServer !== null"
              @click="toggleSecrets(view)"
            >
              <KeyRound :size="10" />
              {{ t("mcp.servers.secrets.title") }}
            </button>
            <button
              type="button"
              data-mcp-server-remove
              class="ml-auto flex items-center gap-1 rounded-md border border-destructive/40 px-2 py-0.5 text-[11px] text-destructive transition-colors hover:bg-destructive/10 disabled:pointer-events-none disabled:opacity-50"
              :disabled="serverDraft !== null || mcp.busyServer !== null"
              @click="removingServer = view"
            >
              <Trash2 :size="10" />
              {{ t("mcp.servers.remove") }}
            </button>
          </div>
          <p class="mt-0.5 text-[11px]" :class="secretsStateClass(view)">{{ secretsStateLabel(view) }}</p>

          <!-- What the server is reached with. Write-only: saving replaces the whole set. -->
          <div
            v-if="secretsFor === view.config.id"
            data-mcp-server-secrets-form
            class="mt-2 space-y-2 rounded-md border border-primary/40 px-3 py-2.5"
          >
            <div>
              <h5 class="text-xs font-medium">{{ t("mcp.servers.secrets.title") }}</h5>
              <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                {{ t("mcp.servers.secrets.description") }}
              </p>
            </div>

            <div class="space-y-1">
              <div class="flex items-center justify-between gap-2">
                <h6 class="text-[11px] font-medium text-muted-foreground">{{ t("mcp.servers.secrets.env") }}</h6>
                <button
                  type="button"
                  class="flex items-center gap-1 rounded-md border border-border px-1.5 py-0.5 text-[11px] transition-colors hover:bg-accent"
                  @click="addSecretRow('env')"
                >
                  <Plus :size="9" />
                  {{ t("mcp.servers.secrets.add") }}
                </button>
              </div>
              <div
                v-for="(row, index) in envRows"
                :key="index"
                data-mcp-server-env-row
                class="grid grid-cols-[1fr_1fr_auto] items-center gap-1.5"
              >
                <input
                  v-model="row.key"
                  data-secret-key
                  type="text"
                  autocomplete="off"
                  spellcheck="false"
                  class="min-w-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] focus:outline-none select-text"
                  :aria-label="t('mcp.servers.secrets.key')"
                  :placeholder="t('mcp.servers.secrets.key')"
                />
                <input
                  v-model="row.value"
                  data-secret-value
                  type="password"
                  autocomplete="off"
                  spellcheck="false"
                  class="min-w-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] focus:outline-none select-text"
                  :aria-label="t('mcp.servers.secrets.value')"
                  :placeholder="t('mcp.servers.secrets.value')"
                />
                <button
                  type="button"
                  class="rounded-md border border-border px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :aria-label="t('mcp.servers.secrets.remove')"
                  :title="t('mcp.servers.secrets.remove')"
                  @click="removeSecretRow('env', index)"
                >
                  <Trash2 :size="10" />
                </button>
              </div>
            </div>

            <div class="space-y-1">
              <div class="flex items-center justify-between gap-2">
                <h6 class="text-[11px] font-medium text-muted-foreground">
                  {{ t("mcp.servers.secrets.headers") }}
                </h6>
                <button
                  type="button"
                  class="flex items-center gap-1 rounded-md border border-border px-1.5 py-0.5 text-[11px] transition-colors hover:bg-accent"
                  @click="addSecretRow('headers')"
                >
                  <Plus :size="9" />
                  {{ t("mcp.servers.secrets.add") }}
                </button>
              </div>
              <div
                v-for="(row, index) in headerRows"
                :key="index"
                data-mcp-server-header-row
                class="grid grid-cols-[1fr_1fr_auto] items-center gap-1.5"
              >
                <input
                  v-model="row.key"
                  data-secret-key
                  type="text"
                  autocomplete="off"
                  spellcheck="false"
                  class="min-w-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] focus:outline-none select-text"
                  :aria-label="t('mcp.servers.secrets.key')"
                  :placeholder="t('mcp.servers.secrets.key')"
                />
                <input
                  v-model="row.value"
                  data-secret-value
                  type="password"
                  autocomplete="off"
                  spellcheck="false"
                  class="min-w-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] focus:outline-none select-text"
                  :aria-label="t('mcp.servers.secrets.value')"
                  :placeholder="t('mcp.servers.secrets.value')"
                />
                <button
                  type="button"
                  class="rounded-md border border-border px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :aria-label="t('mcp.servers.secrets.remove')"
                  :title="t('mcp.servers.secrets.remove')"
                  @click="removeSecretRow('headers', index)"
                >
                  <Trash2 :size="10" />
                </button>
              </div>
            </div>

            <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.secrets.hint") }}</p>
            <p v-if="secretsProblem" class="text-[11px] text-destructive">{{ secretsProblem }}</p>

            <div class="flex flex-wrap items-center gap-2">
              <button
                type="button"
                data-mcp-server-secrets-save
                class="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
                :disabled="mcp.busyServer !== null || secretsProblem !== null"
                @click="saveSecrets()"
              >
                {{ t("mcp.servers.secrets.save") }}
              </button>
              <button
                type="button"
                data-mcp-server-secrets-close
                class="rounded-md border border-border px-3 py-1 text-xs transition-colors hover:bg-accent"
                :disabled="mcp.busyServer !== null"
                @click="secretsFor = null"
              >
                {{ t("mcp.servers.secrets.close") }}
              </button>
              <span v-if="secretsNotice === 'saved'" class="text-xs text-success">
                {{ t("mcp.servers.secrets.saved") }}
              </span>
              <span v-else-if="secretsNotice === 'cleared'" class="text-xs text-muted-foreground">
                {{ t("mcp.servers.secrets.cleared") }}
              </span>
            </div>
          </div>
        </li>
      </ul>

      <!-- The configuration form: one server at a time, with the id fixed once it exists. -->
      <div v-if="serverDraft" data-mcp-server-form class="space-y-2 rounded-md border border-primary/40 px-3 py-2.5">
        <h5 class="text-xs font-medium">
          {{ t(serverDraft.isNew ? "mcp.servers.newTitle" : "mcp.servers.editTitle") }}
        </h5>

        <div class="grid gap-2 sm:grid-cols-2">
          <div>
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-id">
              {{ t("mcp.servers.id") }}
            </label>
            <input
              id="mcp-server-id"
              v-model="serverDraft.id"
              type="text"
              autocomplete="off"
              spellcheck="false"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text disabled:opacity-60"
              :disabled="!serverDraft.isNew"
              :placeholder="t('mcp.servers.id')"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.idHint") }}</p>
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-name">
              {{ t("mcp.servers.name") }}
            </label>
            <input
              id="mcp-server-name"
              v-model="serverDraft.name"
              type="text"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
              :placeholder="t('mcp.servers.name')"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.nameHint") }}</p>
          </div>

          <div>
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-transport">
              {{ t("mcp.servers.transportLabel") }}
            </label>
            <div class="mt-0.5">
              <Select
                id="mcp-server-transport"
                :model-value="serverDraft.transport"
                :options="transportOptions"
                :aria-label="t('mcp.servers.transportLabel')"
                @update:model-value="chooseServerTransport"
              />
            </div>
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-timeout">
              {{ t("mcp.servers.timeout") }}
            </label>
            <div class="mt-0.5 flex items-center gap-2">
              <input
                id="mcp-server-timeout"
                type="number"
                :min="MCP_SERVER_TOOL_TIMEOUT_SECONDS_MIN"
                :max="MCP_SERVER_TOOL_TIMEOUT_SECONDS_MAX"
                step="1"
                class="w-24 rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
                :value="serverDraft.toolTimeoutSeconds"
                @input="typedServerTimeout($event)"
              />
              <span class="text-[11px] text-muted-foreground">{{ t("mcp.fields.unitSeconds") }}</span>
            </div>
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.timeoutHint") }}</p>
          </div>

          <div v-if="serverDraft.transport === 'stdio'" class="sm:col-span-2">
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-command">
              {{ t("mcp.servers.command") }}
            </label>
            <input
              id="mcp-server-command"
              v-model="serverDraft.command"
              type="text"
              autocomplete="off"
              spellcheck="false"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="t('mcp.servers.command')"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.commandHint") }}</p>
          </div>
          <div v-if="serverDraft.transport === 'stdio'" class="sm:col-span-2">
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-args">
              {{ t("mcp.servers.args") }}
            </label>
            <textarea
              id="mcp-server-args"
              v-model="serverDraft.args"
              rows="2"
              spellcheck="false"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="t('mcp.servers.args')"
            ></textarea>
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.argsHint") }}</p>
          </div>
          <div v-else class="sm:col-span-2">
            <label class="block text-[11px] text-muted-foreground" for="mcp-server-url">
              {{ t("mcp.servers.url") }}
            </label>
            <input
              id="mcp-server-url"
              v-model="serverDraft.url"
              type="url"
              autocomplete="off"
              spellcheck="false"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="t('mcp.servers.url')"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.urlHint") }}</p>
          </div>

          <div class="flex items-center justify-between gap-3 sm:col-span-2">
            <span class="text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.servers.enabledHint") }}</span>
            <Switch
              :model-value="serverDraft.enabled"
              :aria-label="t('mcp.servers.enabled')"
              @update:model-value="setDraftEnabled"
            />
          </div>
        </div>

        <p v-if="serverDraftProblem" data-mcp-server-problem class="text-[11px] text-destructive">
          {{ serverDraftProblem }}
        </p>

        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            data-mcp-server-save
            class="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
            :disabled="!canSaveServerDraft || mcp.busyServer !== null"
            @click="saveServerDraft()"
          >
            {{ t("mcp.servers.save") }}
          </button>
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1 text-xs transition-colors hover:bg-accent"
            :disabled="mcp.busyServer !== null"
            @click="cancelServerDraft()"
          >
            {{ t("mcp.servers.cancel") }}
          </button>
        </div>
      </div>

      <p v-if="serverNotice === 'saved'" class="text-xs text-muted-foreground">{{ t("mcp.servers.saved") }}</p>
      <p v-else-if="serverNotice === 'removed'" class="text-xs text-muted-foreground">
        {{ t("mcp.servers.removed") }}
      </p>
    </div>

    <!-- The audit is its own panel: a page that draws the record beside the settings is a page of wall. -->
    <div class="space-y-2">
      <button
        type="button"
        data-mcp-audit-open
        :aria-expanded="auditOpen"
        class="flex w-full items-center gap-1.5 rounded-md border border-border px-3 py-2 text-left text-xs transition-colors hover:bg-accent"
        @click="toggleAudit()"
      >
        <ChevronRight :size="12" class="transition-transform" :class="auditOpen ? 'rotate-90' : undefined" />
        <span class="font-medium">{{ t("mcp.audit.open") }}</span>
        <span class="ml-auto font-mono text-[11px] text-muted-foreground">{{ mcp.audit.length }}</span>
      </button>

      <!-- The tail of the audit file: what was called, how it ended, how long it took. -->
      <div v-if="auditOpen" class="space-y-2 rounded-md border border-border px-3 py-3">
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
                <td class="whitespace-nowrap font-mono text-[11px]" :class="TABLE.page.cell">
                  {{ when(entry.at) }}
                </td>
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
    </div>

    <ConfirmDialog
      v-if="removingServer"
      :open="true"
      :title="t('mcp.servers.removeTitle')"
      :message="t('mcp.servers.removeMessage')"
      :details="[serverName(removingServer)]"
      :confirm-label="t('mcp.servers.remove')"
      danger
      @confirm="confirmRemoveServer()"
      @update:open="removingServer = null"
    />
  </section>
</template>
