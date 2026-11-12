<script setup lang="ts">
/**
 * The capabilities tab: every tool an agent may call, grouped by what it is, with a switch each.
 *
 * Three rules shape it:
 *
 * 1. **The catalog is the host's answer, not a copy of the registry.** Which tools exist, what they
 *    are called, how risky they are and whether they are on all come from `mcp_catalog`. The page
 *    keeps only a draft of the switches, so a tool added by a newer build appears without this file
 *    learning its name.
 * 2. **Switching is a draft, and Apply is the write.** Nothing is sent while a switch moves: the
 *    settings file is written whole, and a write per click would be a dozen files for one decision.
 *    Apply writes `disabledTools` and then re-reads the catalog, so the page draws what the host
 *    now answers rather than what it hoped it wrote.
 * 3. **An external tool is its server's tool first and its own switch second.** The host computes
 *    an external row's state from its server's `enabled` flag and the same `disabledTools` set, so
 *    a row whose server is off stays off here — the switch is disabled and says why — while a row
 *    whose server is on can be switched off individually. The access tab is where the server goes.
 */
import { RefreshCw } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Switch from "@/components/ui/Switch.vue";
import { MCP_TOOL_CATEGORY_LABEL_KEYS, MCP_TOOL_RISK_LABEL_KEYS } from "@/lib/mcp/labels";
import { useMcpStore } from "@/stores/mcp";
import { usePreferencesStore } from "@/stores/preferences";
import {
  MCP_TOOL_CATEGORIES,
  isMcpToolCategory,
  type McpCatalog,
  type McpCatalogTool,
  type McpToolCategory,
  type McpToolRisk,
} from "@/types/mcp";

const { t } = useI18n();
const mcp = useMcpStore();
const preferences = usePreferencesStore();

/** What the last apply did, said under the button. */
const notice = ref<"" | "applied" | "failed">("");
/** What the list is narrowed to. Nothing set means every tool. */
const query = ref("");
const onlyEnabled = ref(false);
/** Whether an apply is in flight: one write, and no second one on top of it. */
const applying = ref(false);

/**
 * The draft: the built-in tools that are off.
 *
 * A set of names rather than a flag per row, because that is exactly what the policy stores — an
 * absent name is an enabled tool, which is also how a tool a newer build added starts out.
 */
const disabled = ref<ReadonlySet<string>>(new Set());

/** How much care a tool needs, drawn as a badge. */
const RISK_BADGE_CLASS: Record<McpToolRisk, string> = {
  read: "border-border text-muted-foreground",
  write: "border-warning/40 text-warning",
  commandGraded: "border-destructive/40 text-destructive",
};

/** A category this build can draw, with an unknown one falling under external rather than vanishing. */
function categoryOf(tool: McpCatalogTool): McpToolCategory {
  return isMcpToolCategory(tool.category) ? tool.category : "external";
}

/**
 * Whether an external tool's server is known to be off.
 *
 * Unknown reads as on: the server list is loaded by the access tab, which is always mounted, and a
 * switch that refused to move because a list had not arrived would be a page that looks broken.
 */
function serverOff(tool: McpCatalogTool): boolean {
  if (tool.source === null) {
    return false;
  }
  const view = mcp.servers.find((entry) => entry.config.id === tool.source);
  return view !== undefined && !view.config.enabled;
}

/** Whether the draft has a tool on, with an external tool's off server outranking its own switch. */
function toolEnabled(tool: McpCatalogTool): boolean {
  if (serverOff(tool)) {
    return false;
  }
  return !disabled.value.has(tool.name);
}

/**
 * Takes the switches from the settings file, plus any built-in the registry reports as off.
 *
 * The file is the draft's base — it holds the names somebody switched off, external ones included.
 * The registry's own answer is added for built-ins because a tool can be off without this window
 * having written it (another window, a settings file edited by hand), and drawing it on would be
 * this page contradicting the thing that refuses the call.
 */
function adoptDraft(catalog: McpCatalog | null): void {
  const next = new Set(preferences.mcpToolPolicy.disabledTools);
  for (const tool of catalog?.tools ?? []) {
    if (tool.source === null && !tool.enabled) {
      next.add(tool.name);
    }
  }
  disabled.value = next;
}

watch(() => mcp.catalog, adoptDraft, { immediate: true });

onMounted(() => {
  void mcp.loadCatalog();
});

/** The tools the two filters leave, in catalog order. */
const shownTools = computed<readonly McpCatalogTool[]>(() => {
  const needle = query.value.trim().toLowerCase();
  return (mcp.catalog?.tools ?? []).filter((tool) => {
    if (onlyEnabled.value && !toolEnabled(tool)) {
      return false;
    }
    if (needle === "") {
      return true;
    }
    return [tool.title, tool.name, tool.description, tool.source ?? ""].some((text) =>
      text.toLowerCase().includes(needle),
    );
  });
});

/** The headings that still have a tool under them, in the page's fixed order. */
const groups = computed(() =>
  MCP_TOOL_CATEGORIES.map((category) => ({
    category,
    tools: shownTools.value.filter((tool) => categoryOf(tool) === category),
  })).filter((group) => group.tools.length > 0),
);

/** Every tool under a heading that a category switch can move: an off server's tools are not its. */
function switchableTools(category: McpToolCategory): readonly McpCatalogTool[] {
  return (mcp.catalog?.tools ?? []).filter((tool) => categoryOf(tool) === category && !serverOff(tool));
}

/** Whether every switchable tool under a heading is on, which is what its switch says. */
function categoryEnabled(category: McpToolCategory): boolean {
  const tools = switchableTools(category);
  return tools.length > 0 && tools.every(toolEnabled);
}

const totalTools = computed(() => mcp.catalog?.tools.length ?? 0);
const enabledTools = computed(() => (mcp.catalog?.tools ?? []).filter(toolEnabled).length);

function setTool(name: string, enabled: boolean): void {
  const next = new Set(disabled.value);
  if (enabled) {
    next.delete(name);
  } else {
    next.add(name);
  }
  disabled.value = next;
  notice.value = "";
}

/** Moves every switchable tool under a heading at once, which is what a heading's switch is for. */
function setCategory(category: McpToolCategory, enabled: boolean): void {
  const next = new Set(disabled.value);
  for (const tool of switchableTools(category)) {
    if (enabled) {
      next.delete(tool.name);
    } else {
      next.add(tool.name);
    }
  }
  disabled.value = next;
  notice.value = "";
}

/**
 * Writes the switches and re-reads the catalog.
 *
 * The names are sorted on the way out so the same set is the same file every time — a policy that
 * reordered itself on every save would be a settings file that always looked modified.
 */
async function apply(): Promise<void> {
  if (applying.value) {
    return;
  }
  notice.value = "";
  applying.value = true;
  preferences.setMcpToolPolicy({ disabledTools: [...disabled.value].sort() });
  try {
    if (!(await preferences.commit())) {
      notice.value = "failed";
      return;
    }
    await mcp.loadCatalog();
    notice.value = "applied";
  } finally {
    applying.value = false;
  }
}

function refresh(): void {
  notice.value = "";
  void mcp.loadCatalog();
}

function badgeClass(risk: McpToolRisk): string {
  return RISK_BADGE_CLASS[risk];
}
</script>

<template>
  <section class="space-y-3" data-mcp-capabilities>
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h4 class="text-sm font-medium">{{ t("mcp.capabilities.title") }}</h4>
        <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          {{ t("mcp.capabilities.description") }}
        </p>
      </div>
      <button
        type="button"
        data-mcp-catalog-refresh
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
        :disabled="mcp.loadingCatalog"
        @click="refresh()"
      >
        <RefreshCw :size="12" :class="mcp.loadingCatalog ? 'animate-spin' : undefined" />
        {{ t("mcp.capabilities.refresh") }}
      </button>
    </div>

    <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
      <input
        data-mcp-capabilities-search
        :value="query"
        type="search"
        autocomplete="off"
        spellcheck="false"
        class="min-w-48 flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-xs focus:outline-none select-text"
        :aria-label="t('mcp.capabilities.search')"
        :placeholder="t('mcp.capabilities.search')"
        @input="query = ($event.target as HTMLInputElement).value"
      />
      <label class="flex items-center gap-2 text-xs text-muted-foreground">
        <Switch
          data-mcp-capabilities-filter
          :model-value="onlyEnabled"
          :aria-label="t('mcp.capabilities.onlyEnabled')"
          @update:model-value="(value: boolean) => (onlyEnabled = value)"
        />
        {{ t("mcp.capabilities.onlyEnabled") }}
      </label>
      <span v-if="mcp.catalog" data-mcp-counts class="text-xs text-muted-foreground">
        {{ t("mcp.capabilities.counts", { enabled: String(enabledTools), total: String(totalTools) }) }}
      </span>
    </div>

    <p v-if="mcp.catalog === null" class="text-xs text-muted-foreground">
      {{ mcp.catalogLoaded ? t("mcp.capabilities.empty") : t("mcp.capabilities.loading") }}
    </p>
    <p v-else-if="groups.length === 0" class="text-xs text-muted-foreground">{{ t("mcp.capabilities.empty") }}</p>

    <section v-for="group in groups" :key="group.category" class="space-y-1.5">
      <div class="flex items-center justify-between gap-3">
        <h5 class="text-xs font-medium text-muted-foreground">
          {{ t(MCP_TOOL_CATEGORY_LABEL_KEYS[group.category]) }}
          <span class="ml-1 font-mono text-[11px]">{{ group.tools.length }}</span>
        </h5>
        <Switch
          v-if="switchableTools(group.category).length > 0"
          :data-mcp-category-toggle="group.category"
          :model-value="categoryEnabled(group.category)"
          :aria-label="t(MCP_TOOL_CATEGORY_LABEL_KEYS[group.category])"
          @update:model-value="(value: boolean) => setCategory(group.category, value)"
        />
      </div>

      <ul class="space-y-1">
        <li
          v-for="tool in group.tools"
          :key="tool.name"
          data-mcp-tool
          :data-mcp-tool-name="tool.name"
          :data-mcp-tool-source="tool.source ?? 'builtin'"
          :data-mcp-tool-enabled="toolEnabled(tool)"
          class="rounded-md border border-border bg-muted/20 px-3 py-2"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                <span class="text-xs font-medium">{{ tool.title }}</span>
                <span class="font-mono text-[11px] text-muted-foreground">{{ tool.name }}</span>
                <span
                  data-mcp-risk
                  :data-mcp-risk-level="tool.risk"
                  class="rounded border px-1 py-px text-[11px]"
                  :class="badgeClass(tool.risk)"
                >
                  {{ t(MCP_TOOL_RISK_LABEL_KEYS[tool.risk]) }}
                </span>
                <span v-if="tool.source" class="text-[11px] text-muted-foreground">
                  {{ t("mcp.capabilities.source", { source: tool.source }) }}
                </span>
                <span
                  v-if="serverOff(tool)"
                  data-mcp-server-off
                  class="rounded border border-warning/40 px-1 py-px text-[11px] text-warning"
                >
                  {{ t("mcp.capabilities.serverOff") }}
                </span>
              </div>
              <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{{ tool.description }}</p>
            </div>
            <Switch
              data-mcp-tool-switch
              :model-value="toolEnabled(tool)"
              :disabled="serverOff(tool)"
              :aria-label="tool.title"
              @update:model-value="(value: boolean) => setTool(tool.name, value)"
            />
          </div>
        </li>
      </ul>

      <p v-if="group.category === 'external'" class="text-[11px] leading-relaxed text-muted-foreground">
        {{ t("mcp.capabilities.externalHint") }}
      </p>
    </section>

    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        data-mcp-apply="capabilities"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
        :disabled="applying || mcp.catalog === null"
        @click="apply()"
      >
        {{ t("mcp.capabilities.apply") }}
      </button>
      <span v-if="notice === 'applied'" data-mcp-applied="capabilities" class="text-xs text-success">{{
        t("mcp.capabilities.applied")
      }}</span>
      <span v-else-if="notice === 'failed'" class="text-xs text-destructive">
        {{ t("mcp.capabilities.failed") }}
      </span>
    </div>
  </section>
</template>
