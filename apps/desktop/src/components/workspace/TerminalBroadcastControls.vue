<script setup lang="ts">
import { ChevronDown, Radio, Square } from "@lucide/vue";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { sessionEndpoint } from "@/lib/menu/sessions";
import { sessionTabTitle } from "@/lib/sessions/reopen";
import { useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const terminals = useTerminalsStore();
const open = ref(false);
const enabled = computed(() => terminals.broadcast?.enabled === true);
const count = computed(() => terminals.broadcast?.paneIds.length ?? 0);
const names = computed(() =>
  terminals.broadcastPanes
    .filter((pane) => terminals.broadcast?.paneIds.includes(pane.id))
    .map((pane) => sessionTabTitle(pane.session.name, terminals.copyOf(pane.id)))
    .join(" · "),
);

function show(value: boolean): void {
  if (value) terminals.prepareBroadcast();
  open.value = value;
}

function start(): void {
  if (terminals.startBroadcast()) open.value = false;
}
</script>

<template>
  <div
    data-terminal-broadcast
    :data-enabled="enabled"
    class="flex min-h-8 shrink-0 items-center gap-2 border-b px-2 py-1 text-[11px]"
    :class="enabled ? 'border-warning/40 bg-warning/10' : 'border-border bg-card/30'"
  >
    <span class="sr-only" role="status">{{
      enabled ? t("terminal.broadcast.active", { count }) : t("terminal.broadcast.off")
    }}</span>
    <PopoverRoot :open="open" @update:open="show">
      <PopoverTrigger as-child>
        <button
          type="button"
          data-broadcast-open
          :aria-label="t('terminal.broadcast.configure')"
          :aria-expanded="open"
          class="flex shrink-0 items-center gap-1.5 rounded-sm px-1.5 py-0.5 font-medium hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <Radio :size="13" :class="enabled ? 'text-warning' : 'text-muted-foreground'" />
          {{ enabled ? t("terminal.broadcast.active", { count }) : t("terminal.broadcast.title") }}
          <ChevronDown :size="11" class="text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverPortal>
        <PopoverContent
          data-broadcast-picker
          side="bottom"
          align="start"
          :side-offset="6"
          :collision-padding="8"
          class="z-50 flex max-h-[min(28rem,70vh)] w-[min(24rem,calc(100vw-2rem))] flex-col rounded-lg border border-border bg-card shadow-xl"
        >
          <div class="border-b border-border px-3 py-2.5">
            <p class="text-xs font-medium">{{ t("terminal.broadcast.configure") }}</p>
            <p class="mt-1 text-[11px] text-muted-foreground">{{ t("terminal.broadcast.hint") }}</p>
          </div>
          <div class="flex items-center justify-between px-3 pt-2 text-[11px]">
            <span class="text-muted-foreground">{{ t("terminal.broadcast.targets", { count }) }}</span>
            <button
              type="button"
              data-broadcast-all
              class="rounded-sm px-1.5 py-0.5 hover:bg-accent"
              @click="terminals.selectAllBroadcastPanes()"
            >
              {{ t("terminal.broadcast.selectAll") }}
            </button>
          </div>
          <div class="min-h-0 overflow-auto p-1.5">
            <label
              v-for="pane in terminals.broadcastPanes"
              :key="pane.id"
              class="flex items-center gap-2 rounded-md px-2 py-2 hover:bg-accent/50"
              :class="pane.status !== 'open' || pane.handle === null ? 'opacity-50' : 'cursor-pointer'"
            >
              <input
                type="checkbox"
                data-broadcast-target
                :data-pane-id="pane.id"
                :checked="terminals.broadcast?.paneIds.includes(pane.id)"
                :disabled="pane.status !== 'open' || pane.handle === null"
                :aria-label="
                  t('terminal.broadcast.target', {
                    name: sessionTabTitle(pane.session.name, terminals.copyOf(pane.id)),
                  })
                "
                class="size-3.5 shrink-0 accent-primary"
                @change="terminals.setBroadcastPane(pane.id, ($event.target as HTMLInputElement).checked)"
              />
              <span class="min-w-0 flex-1">
                <span class="block truncate text-xs">{{
                  sessionTabTitle(pane.session.name, terminals.copyOf(pane.id))
                }}</span>
                <span class="block truncate font-mono text-[11px] text-muted-foreground">{{
                  sessionEndpoint(pane.session, t)
                }}</span>
              </span>
              <span v-if="pane.status !== 'open'" class="text-[11px] text-muted-foreground">{{
                t("terminal.broadcast.unavailable")
              }}</span>
            </label>
          </div>
          <div class="border-t border-border px-3 py-2.5">
            <p class="mb-2 text-[11px] text-muted-foreground">{{ t("terminal.broadcast.selectionHint") }}</p>
            <button
              v-if="!enabled"
              type="button"
              data-broadcast-start
              class="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-40"
              :disabled="count < 2"
              @click="start"
            >
              <Radio :size="13" />{{ t("terminal.broadcast.start") }}
            </button>
            <button
              v-else
              type="button"
              class="w-full rounded-md border border-warning/40 px-3 py-1.5 text-xs hover:bg-warning/10"
              @click="
                terminals.stopBroadcast();
                open = false;
              "
            >
              {{ t("terminal.broadcast.stop") }}
            </button>
          </div>
        </PopoverContent>
      </PopoverPortal>
    </PopoverRoot>
    <span v-if="enabled" class="min-w-0 flex-1 truncate text-muted-foreground" :title="names">{{ names }}</span>
    <span v-else class="min-w-0 flex-1 truncate text-muted-foreground">{{ t("terminal.broadcast.off") }}</span>
    <button
      v-if="enabled"
      type="button"
      data-broadcast-stop
      class="flex shrink-0 items-center gap-1 rounded-sm border border-warning/40 px-2 py-0.5 hover:bg-warning/10"
      @click="terminals.stopBroadcast()"
    >
      <Square :size="10" />{{ t("terminal.broadcast.stop") }}
    </button>
  </div>
</template>
