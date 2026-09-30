<script setup lang="ts">
/**
 * The session list, and the protocols this build actually speaks.
 *
 * There is no session list yet, and the sidebar says so instead of showing an empty box: the
 * empty state explains what will fill it. The protocol section is not a placeholder — it is the
 * descriptor table, which is the same data Rust reads.
 */
import { Plus, Server, Terminal } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { useAppStore } from "@/stores/app";
import { PROTOCOLS } from "@/types/generated/protocols";

const { t } = useI18n();
const app = useAppStore();

const protocols = computed(() => PROTOCOLS);

const statusLabel = computed(() => {
  switch (app.status) {
    case "loading":
      return t("common.status.loading");
    case "failed":
      return t("common.status.failed");
    default:
      return t("common.status.ready");
  }
});

const statusDot = computed(() => {
  if (app.status === "failed") {
    return "bg-destructive";
  }
  return app.status === "ready" ? "bg-success" : "bg-warning";
});
</script>

<template>
  <aside class="flex w-64 shrink-0 flex-col border-r border-border bg-card/30">
    <section class="px-3 pt-3">
      <header class="mb-2 flex items-center justify-between">
        <h2 class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("shell.sidebar.sessions") }}
        </h2>
        <button
          type="button"
          disabled
          class="grid size-6 cursor-not-allowed place-items-center rounded-md text-muted-foreground/50"
          :title="t('shell.sidebar.newSessionPending')"
        >
          <Plus :size="14" />
        </button>
      </header>

      <div class="rounded-md border border-dashed border-border px-3 py-4 text-center">
        <Server :size="18" class="mx-auto mb-2 text-muted-foreground" />
        <p class="text-xs leading-relaxed text-muted-foreground">{{ t("shell.sidebar.empty") }}</p>
      </div>
    </section>

    <section class="mt-5 px-3">
      <h2 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("shell.sidebar.protocols") }}
      </h2>
      <ul class="space-y-0.5">
        <li
          v-for="protocol in protocols"
          :key="protocol.id"
          class="flex items-center justify-between rounded-md px-2 py-1.5 text-sm hover:bg-accent/60"
        >
          <span class="flex items-center gap-2">
            <Terminal :size="14" class="text-muted-foreground" />
            {{ t(protocol.labelKey) }}
          </span>
          <span class="font-mono text-[10px] text-muted-foreground">{{ protocol.defaultPort ?? "—" }}</span>
        </li>
      </ul>
    </section>

    <div class="mt-auto flex items-center gap-2 px-4 py-3 text-[11px] text-muted-foreground">
      <span class="size-1.5 rounded-full" :class="statusDot"></span>
      {{ statusLabel }}
    </div>
  </aside>
</template>
