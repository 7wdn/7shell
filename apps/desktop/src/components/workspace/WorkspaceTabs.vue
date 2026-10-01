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
 *
 * A terminal tab can also carry the file panel, which is the second half of the mode: the terminal
 * above, the browser below, and a line between them that can be dragged. Everything the terminal
 * owns — its status strip and its endings — stays above that line, so what is below it is only ever
 * the panel; the drag measures from the bottom of the tab, and that has to keep meaning "the
 * bottom of the panel".
 *
 * The third kind is a page about a host rather than a thing connected to one — the monitor's
 * system, process and network pages. It draws a host's own shape in the strip (its title and the
 * machine it is about) instead of a session name, and it has no status dot: whether it is being
 * read is a question about a terminal it does not own.
 */
import { CircleAlert, Cpu, Folder, List, LoaderCircle, Network, PanelBottom, RotateCw, Terminal, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";

import FileBrowser from "@/components/files/FileBrowser.vue";
import HostPage from "@/components/inspect/HostPage.vue";
import NetworkReport from "@/components/inspect/NetworkReport.vue";
import ProcessTable from "@/components/inspect/ProcessTable.vue";
import SystemReport from "@/components/inspect/SystemReport.vue";
import TerminalPane from "@/components/terminal/TerminalPane.vue";
import SplitDivider from "@/components/workspace/SplitDivider.vue";
import { describeShellError } from "@/i18n/errors";
import { MIN_PANE_PX } from "@/lib/workspace/split";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/stores/preferences";
import { type TerminalTab, type WorkspaceTab, useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const preferences = usePreferencesStore();
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
  // A page about a host has no status of its own: whether it is being read is the page's own
  // business, and a dot would be reporting a connection that belongs to a terminal somewhere else.
  if (tab.kind === "inspect") {
    return null;
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
          <Folder v-else-if="tab.kind === 'files'" :size="12" class="shrink-0" />
          <Cpu v-else-if="tab.view === 'system'" :size="12" class="shrink-0" />
          <List v-else-if="tab.view === 'processes'" :size="12" class="shrink-0" />
          <Network v-else :size="12" class="shrink-0" />
          <!--
            A page names the machine it is about rather than only its kind: two hosts' system pages
            look identical in a tab strip otherwise, and which machine a tab is showing is exactly
            what somebody reads a strip for.
          -->
          <span v-if="tab.kind === 'inspect' && tab.view === 'system'" class="truncate">
            {{ t("inspect.system.title") }} · {{ tab.session.host }}
          </span>
          <span v-else-if="tab.kind === 'inspect' && tab.view === 'processes'" class="truncate">
            {{ t("inspect.processes.title") }} · {{ tab.session.host }}
          </span>
          <span v-else-if="tab.kind === 'inspect'" class="truncate">
            {{ t("inspect.network.title") }} · {{ tab.session.host }}
          </span>
          <span v-else class="truncate">{{ tab.session.name }}</span>
        </button>
        <button
          type="button"
          class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="
            tab.kind === 'files'
              ? t('files.tab.close')
              : tab.kind === 'inspect'
                ? t('inspect.tab.close')
                : t('terminal.tab.close')
          "
          :aria-label="
            tab.kind === 'files'
              ? t('files.tab.close')
              : tab.kind === 'inspect'
                ? t('inspect.tab.close')
                : t('terminal.tab.close')
          "
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
        <!--
          The terminal carries the same floor as the panel while a panel is under it. Without it,
          the height somebody chose in a large window would eat the whole terminal in a small one:
          the panel shrinks first, and it shrinks past the point where the terminal is still there.
          With no panel open the floor is left off, because then the terminal is the only thing
          asking for room and squeezing it is exactly what the window shrinking means.
        -->
        <TerminalPane
          v-if="tab.handle"
          :key="`${tab.id}:${tab.handle}`"
          :handle="tab.handle"
          :active="tab.id === terminals.activeTabId"
          :interactive="tab.status === 'open'"
          class="min-h-0 flex-1"
          :style="tab.panel ? { minHeight: `${MIN_PANE_PX}px` } : undefined"
        />

        <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <CircleAlert :size="18" class="text-destructive" />
          <p class="text-xs text-muted-foreground">{{ t("terminal.tab.failedTitle") }}</p>
        </div>

        <!--
          The terminal's own strip: where it is, and the way into the file panel. It stays when the
          panel is open — it is where the toggle that closes it lives — and it sits above the
          divider rather than below it, because what is under the line is the panel and nothing else.
        -->
        <footer class="flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1">
          <span class="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground">
            {{ tab.session.user }}@{{ tab.session.host }}:{{ tab.session.port }}
          </span>
          <button
            type="button"
            :class="
              cn(
                'flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] transition-colors',
                tab.panel ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground',
                'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent',
              )
            "
            :disabled="tab.panel === null && tab.status !== 'open'"
            :aria-pressed="tab.panel !== null"
            :title="tab.panel ? t('terminal.panel.close') : t('terminal.panel.open')"
            @click="terminals.togglePanel(tab)"
          >
            <PanelBottom :size="11" />
            {{ t("terminal.panel.label") }}
          </button>
        </footer>

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

        <template v-if="tab.panel">
          <SplitDivider
            :height="preferences.resolvedPanelHeight"
            @move="preferences.setPanelHeight($event)"
            @settle="preferences.rememberPanelHeight()"
            @reset="preferences.resetPanelHeight()"
          />

          <!--
            The panel is the only thing below the line, and the floor is a minimum height rather
            than a clamp computed here: a window too short for the height somebody chose shrinks
            the panel, because the half that has to stay usable is the terminal.
          -->
          <div
            class="flex shrink flex-col overflow-hidden"
            :style="{ height: `${preferences.resolvedPanelHeight}px`, minHeight: `${MIN_PANE_PX}px` }"
          >
            <FileBrowser v-if="tab.panel.handle" :handle="tab.panel.handle" class="min-h-0 flex-1" />

            <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
              <CircleAlert v-if="tab.panel.status === 'failed'" :size="18" class="text-destructive" />
              <LoaderCircle v-else :size="18" class="animate-spin text-muted-foreground" />
              <p v-if="tab.panel.failure" class="text-xs">
                {{ describeShellError(translate, tab.panel.failure).message }}
              </p>
              <p v-if="tab.panel.failure" class="font-mono text-[10px] text-muted-foreground">
                {{ describeShellError(translate, tab.panel.failure).code }}
              </p>
              <p v-else class="text-xs text-muted-foreground">{{ t("terminal.panel.opening") }}</p>
              <button
                v-if="tab.panel.status === 'failed'"
                type="button"
                class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
                @click="terminals.openPanel(tab)"
              >
                <RotateCw :size="12" />
                {{ t("terminal.panel.retry") }}
              </button>
            </div>

            <!--
              A browser that refused to close is still open, so it is still drawn — with the reason
              underneath it, because this is the one failure that leaves the panel working.
            -->
            <footer
              v-if="tab.panel.handle && tab.panel.failure"
              class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-3 py-1.5"
            >
              <p class="text-[11px]">{{ describeShellError(translate, tab.panel.failure).message }}</p>
              <p class="font-mono text-[10px] text-muted-foreground">
                {{ describeShellError(translate, tab.panel.failure).code }}
              </p>
            </footer>
          </div>
        </template>
      </template>

      <template v-else-if="tab.kind === 'files'">
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

      <!--
        A page about the host. It is handed the session rather than a handle, and the frame decides
        whether there is a connection to read through — which is why a page can be left open across
        a reconnect and why one whose host is not connected is a page waiting rather than a failure.
        The process page is handed whether it is the tab on screen as well, because it reads a list
        of its own and should do that only while somebody is looking at it.
      -->
      <template v-else>
        <HostPage :session="tab.session" :view="tab.view" :active="tab.id === terminals.activeTabId">
          <template #default="{ live }">
            <SystemReport v-if="tab.view === 'system'" :live="live" />
            <ProcessTable v-else-if="tab.view === 'processes'" :active="tab.id === terminals.activeTabId" />
            <NetworkReport v-else :live="live" :active="tab.id === terminals.activeTabId" />
          </template>
        </HostPage>
      </template>
    </div>
  </section>
</template>
