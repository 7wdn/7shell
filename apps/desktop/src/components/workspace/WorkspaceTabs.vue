<script setup lang="ts">
/**
 * The workspace: a tab per terminal and per file browser, and what state each one is in.
 *
 * A terminal tab is not dropped when its terminal ends. The reason the server gave, and a way to
 * connect again, are things only that tab can offer — closing it on someone's behalf would take
 * both away and leave a workspace that quietly lost a session.
 *
 * Panes are kept mounted and hidden rather than created on demand, so switching tabs does not throw
 * away a terminal's scrollback — and so the pane that owns a handle is always the one the terminal
 * bus has registered. A file browser keeps its selection and its typed path the same way.
 *
 * The tab itself is one union with two shapes, so the bar draws both from one list: the icon says
 * which kind it is, and only the tabs with a stream that can end carry a status dot.
 */
import { CircleAlert, Folder, RotateCw, Terminal, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";

import FileBrowser from "@/components/files/FileBrowser.vue";
import TerminalPane from "@/components/terminal/TerminalPane.vue";
import { describeShellError } from "@/i18n/errors";
import { cn } from "@/lib/utils";
import { type TerminalTab, type WorkspaceTab, useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const terminals = useTerminalsStore();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function translateStatus(tab: TerminalTab): string | null {
  if (tab.status !== "ended") {
    return null;
  }
  return tab.reason ? t("terminal.tab.endedReason", { reason: tab.reason }) : t("terminal.tab.endedNoReason");
}

/** A file browser has no stream to end, so among those only a failed one has a dot to show. */
function dotClass(tab: WorkspaceTab): string | null {
  if (tab.kind === "files") {
    return tab.status === "failed" ? "bg-destructive" : null;
  }
  switch (tab.status) {
    case "failed":
      return "bg-destructive";
    case "ended":
      return "bg-warning";
    default:
      return "bg-success";
  }
}
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 items-stretch gap-1 overflow-x-auto border-b border-border bg-card/30 px-2 pt-1.5">
      <div
        v-for="tab in terminals.tabs"
        :key="tab.id"
        :class="
          cn(
            'flex max-w-60 shrink-0 items-center gap-1 rounded-t-md border border-b-0 px-2',
            tab.id === terminals.activeTabId
              ? 'border-border bg-background'
              : 'border-transparent text-muted-foreground hover:bg-accent/60',
          )
        "
      >
        <button
          type="button"
          class="flex min-w-0 items-center gap-2 py-1.5 text-xs"
          @click="terminals.activate(tab.id)"
        >
          <span v-if="dotClass(tab)" class="size-1.5 shrink-0 rounded-full" :class="dotClass(tab)"></span>
          <Terminal v-if="tab.kind === 'terminal'" :size="12" class="shrink-0" />
          <Folder v-else :size="12" class="shrink-0" />
          <span class="truncate">{{ tab.session.name }}</span>
        </button>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="tab.kind === 'files' ? t('files.tab.close') : t('terminal.tab.close')"
          :aria-label="tab.kind === 'files' ? t('files.tab.close') : t('terminal.tab.close')"
          @click="terminals.closeTab(tab.id)"
        >
          <X :size="11" />
        </button>
      </div>
    </div>

    <div
      v-for="tab in terminals.tabs"
      v-show="tab.id === terminals.activeTabId"
      :key="tab.id"
      class="flex min-h-0 flex-1 flex-col"
    >
      <template v-if="tab.kind === 'terminal'">
        <TerminalPane
          v-if="tab.handle"
          :key="`${tab.id}:${tab.handle}`"
          :handle="tab.handle"
          :active="tab.id === terminals.activeTabId"
          :interactive="tab.status === 'open'"
          class="min-h-0 flex-1"
        />

        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <CircleAlert :size="18" class="text-destructive" />
          <p class="text-xs text-muted-foreground">{{ t("terminal.tab.failedTitle") }}</p>
        </div>

        <footer
          v-if="tab.status === 'ended' || tab.failure"
          :class="
            cn(
              'shrink-0 border-t px-4 py-2',
              tab.failure ? 'border-destructive/40 bg-destructive/10' : 'border-border bg-muted/40',
            )
          "
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="min-w-0">
              <p v-if="tab.status === 'ended'" class="text-xs">{{ t("terminal.tab.ended") }}</p>
              <p v-if="translateStatus(tab)" class="text-[11px] text-muted-foreground">{{ translateStatus(tab) }}</p>
              <p v-if="tab.failure" class="text-xs">{{ describeShellError(translate, tab.failure).message }}</p>
              <p v-if="tab.failure" class="font-mono text-[10px] text-muted-foreground">
                {{ describeShellError(translate, tab.failure).code }}
              </p>
            </div>
            <button
              v-if="tab.status !== 'open'"
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
              @click="terminals.reconnect(tab.id)"
            >
              <RotateCw :size="12" />
              {{ t("terminal.tab.reconnect") }}
            </button>
          </div>
        </footer>
      </template>

      <template v-else>
        <FileBrowser v-if="tab.handle" :handle="tab.handle" class="min-h-0 flex-1" />

        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <CircleAlert :size="18" class="text-destructive" />
          <p class="text-xs text-muted-foreground">{{ t("files.tab.failedTitle") }}</p>
        </div>

        <footer
          v-if="tab.status === 'failed'"
          class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-4 py-2"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="min-w-0">
              <p v-if="tab.failure" class="text-xs">{{ describeShellError(translate, tab.failure).message }}</p>
              <p v-if="tab.failure" class="font-mono text-[10px] text-muted-foreground">
                {{ describeShellError(translate, tab.failure).code }}
              </p>
            </div>
            <!-- A browser that already has a handle cannot be opened again: a retry would be a
                 second browser on the same session. Only a failed open is retryable. -->
            <button
              v-if="tab.handle === null"
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
              @click="terminals.reconnect(tab.id)"
            >
              <RotateCw :size="12" />
              {{ t("files.tab.retry") }}
            </button>
          </div>
        </footer>
      </template>
    </div>
  </section>
</template>
