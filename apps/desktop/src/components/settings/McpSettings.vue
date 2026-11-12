<script setup lang="ts">
/**
 * Agent access, in three pages: how a client gets in, what an agent may call, and what each saved
 * session allows.
 *
 * The split is not decoration. The three answer different questions and change at different speeds:
 * the listener is a command with a live status behind it, the tool switches are a draft written by
 * Apply, and the permission tree is another draft over the session library. Drawing them as one
 * column is what made the page read as a wall of checkboxes with a log at the bottom.
 *
 * What stays here is the shell: the last failure, the tab strip, and the guarantee that a panel is
 * mounted once and then kept. Keeping them mounted is what makes a draft survive a look at another
 * tab — switching back finds the switches where they were left, not reset by the visit.
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import McpAccessPanel from "@/components/settings/McpAccessPanel.vue";
import McpCapabilitiesPanel from "@/components/settings/McpCapabilitiesPanel.vue";
import McpPermissionsPanel from "@/components/settings/McpPermissionsPanel.vue";
import { describeShellError } from "@/i18n/errors";
import { useMcpStore } from "@/stores/mcp";

const TABS = [
  { id: "access", labelKey: "mcp.tabs.access" },
  { id: "capabilities", labelKey: "mcp.tabs.capabilities" },
  { id: "permissions", labelKey: "mcp.tabs.permissions" },
] as const;

type TabId = (typeof TABS)[number]["id"];

const { t } = useI18n();
const mcp = useMcpStore();

/** Which page is on screen. Access is the one a fresh install needs. */
const tab = ref<TabId>("access");

/**
 * The panels that have been shown at least once.
 *
 * A draft dies with its component, so a page left for another tab has to stay mounted behind
 * `v-show` rather than be torn down. The set rather than three flags: it is the same answer for all
 * three, and a fourth tab would need no new field.
 */
const visited = ref<ReadonlySet<TabId>>(new Set<TabId>(["access"]));

const problem = computed(() => (mcp.failure ? describeShellError(translate, mcp.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function choose(next: TabId): void {
  tab.value = next;
  if (!visited.value.has(next)) {
    visited.value = new Set([...visited.value, next]);
  }
}
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

    <!-- The three questions, in the order somebody asks them. -->
    <div class="flex gap-1 rounded-md border border-border bg-muted/30 p-1" role="tablist">
      <button
        v-for="entry in TABS"
        :key="entry.id"
        type="button"
        role="tab"
        :aria-selected="tab === entry.id"
        :data-mcp-tab="entry.id"
        class="flex-1 rounded px-3 py-1.5 text-xs transition-colors"
        :class="tab === entry.id ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
        @click="choose(entry.id)"
      >
        {{ t(entry.labelKey) }}
      </button>
    </div>

    <!--
      `v-show` rather than `v-if`: a panel that has been opened stays mounted, so an unapplied draft
      on the capabilities or permissions page survives a look at how a client connects.
    -->
    <div v-show="tab === 'access'" role="tabpanel" data-mcp-panel="access">
      <McpAccessPanel />
    </div>
    <div v-show="tab === 'capabilities'" role="tabpanel" data-mcp-panel="capabilities">
      <McpCapabilitiesPanel v-if="visited.has('capabilities')" />
    </div>
    <div v-show="tab === 'permissions'" role="tabpanel" data-mcp-panel="permissions">
      <McpPermissionsPanel v-if="visited.has('permissions')" />
    </div>
  </section>
</template>
