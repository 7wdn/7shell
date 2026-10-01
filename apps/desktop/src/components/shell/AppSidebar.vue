<script setup lang="ts">
/**
 * The session list, the forwards that are up, and the protocols this build speaks.
 *
 * The list is what is saved, grouped by the groups that are saved. Nothing here invents a row: a
 * session whose group no longer exists still appears, under the ungrouped heading, rather than
 * disappearing because its heading did.
 *
 * Opening a terminal hangs off the row itself, with the file browser beside it; whether a terminal
 * comes up with its file panel is the settings' answer, not this row's. Editing and deleting are
 * separate buttons, and deleting always asks first, in the row, before anything is removed.
 *
 * The tunnels section draws what the terminals store was told by the backend, and only then: an
 * empty section is not shown at all. A forward that failed to come up is shown next to the ones
 * that are, because "the tunnel is not there" is the answer somebody staring at a port needs.
 */
import { Folder, LoaderCircle, Network, Pencil, Plus, Server, Terminal, Trash2, TriangleAlert, X } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import SessionDialog from "@/components/sessions/SessionDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { useAppStore } from "@/stores/app";
import { useSessionsStore } from "@/stores/sessions";
import { tunnelFailureKey, useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";
import { PROTOCOLS } from "@/types/generated/protocols";

const { t } = useI18n();
const app = useAppStore();
const library = useSessionsStore();
const terminals = useTerminalsStore();

const protocols = computed(() => PROTOCOLS);
const editorOpen = ref(false);
const editing = ref<Session | null>(null);
/** The session whose delete confirmation is open, if any. Never a bare click. */
const confirming = ref<string | null>(null);

const problem = computed(() => (library.failure ? describeShellError(translate, library.failure) : null));
const tunnelProblem = computed(() =>
  terminals.tunnelProblem ? describeShellError(translate, terminals.tunnelProblem) : null,
);
const hasTunnels = computed(
  () => terminals.tunnels.length > 0 || terminals.tunnelFailures.length > 0 || tunnelProblem.value !== null,
);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function endpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

function startNew(): void {
  editing.value = null;
  editorOpen.value = true;
}

function startEdit(session: Session): void {
  editing.value = session;
  editorOpen.value = true;
}

async function remove(session: Session): Promise<void> {
  confirming.value = null;
  await library.remove(session.id);
}

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
    <section class="flex min-h-0 flex-1 flex-col px-3 pt-3">
      <header class="mb-2 flex shrink-0 items-center justify-between">
        <h2 class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("shell.sidebar.sessions") }}
        </h2>
        <button
          type="button"
          class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('sessions.list.add')"
          :aria-label="t('sessions.list.add')"
          @click="startNew()"
        >
          <Plus :size="14" />
        </button>
      </header>

      <p v-if="problem" class="mb-2 shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
        <span class="block text-[11px]">{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>

      <div class="min-h-0 flex-1 overflow-y-auto">
        <p
          v-if="library.status === 'loading' && library.sessions.length === 0"
          class="px-1 py-2 text-xs text-muted-foreground"
        >
          {{ t("common.status.loading") }}
        </p>

        <div
          v-else-if="library.sessions.length === 0"
          class="rounded-md border border-dashed border-border px-3 py-4 text-center"
        >
          <Server :size="18" class="mx-auto mb-2 text-muted-foreground" />
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sessions.list.empty") }}</p>
        </div>

        <template v-else>
          <section v-for="section in library.sections" :key="section.key" class="mb-3">
            <h3
              v-if="section.name || library.groups.length > 0"
              class="mb-1 px-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
            >
              {{ section.name ?? t("sessions.list.ungrouped") }}
            </h3>

            <ul class="space-y-0.5">
              <li v-for="session in section.sessions" :key="session.id">
                <div class="group flex items-center gap-0.5 rounded-md hover:bg-accent/60">
                  <button
                    type="button"
                    class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left"
                    :title="t('sessions.list.open', { name: session.name })"
                    @click="terminals.open(session)"
                  >
                    <Terminal :size="13" class="shrink-0 text-muted-foreground" />
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-sm">{{ session.name }}</span>
                      <span class="block truncate font-mono text-[10px] text-muted-foreground">{{
                        endpoint(session)
                      }}</span>
                    </span>
                    <LoaderCircle
                      v-if="terminals.connectingSessionIds.includes(session.id)"
                      :size="12"
                      class="shrink-0 animate-spin text-muted-foreground"
                      :aria-label="t('sessions.list.connecting', { name: session.name })"
                    />
                  </button>

                  <button
                    type="button"
                    class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                    :title="t('files.open', { name: session.name })"
                    :aria-label="t('files.open', { name: session.name })"
                    @click="terminals.openFiles(session)"
                  >
                    <Folder :size="12" />
                  </button>
                  <button
                    type="button"
                    class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                    :title="t('sessions.list.edit')"
                    :aria-label="t('sessions.list.edit')"
                    @click="startEdit(session)"
                  >
                    <Pencil :size="12" />
                  </button>
                  <button
                    type="button"
                    class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
                    :title="t('sessions.list.remove')"
                    :aria-label="t('sessions.list.remove')"
                    @click="confirming = session.id"
                  >
                    <Trash2 :size="12" />
                  </button>
                </div>

                <div
                  v-if="confirming === session.id"
                  class="mt-1 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-2"
                >
                  <p class="text-[11px]">{{ t("sessions.list.confirmRemove", { name: session.name }) }}</p>
                  <p class="mt-1 text-[10px] leading-relaxed text-muted-foreground">
                    {{ t("sessions.list.confirmRemoveNote") }}
                  </p>
                  <div class="mt-2 flex gap-1">
                    <button
                      type="button"
                      class="rounded-md bg-destructive px-2 py-1 text-[11px] text-destructive-foreground transition-colors hover:bg-destructive/90"
                      @click="remove(session)"
                    >
                      {{ t("sessions.list.confirm") }}
                    </button>
                    <button
                      type="button"
                      class="rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
                      @click="confirming = null"
                    >
                      {{ t("sessions.list.cancel") }}
                    </button>
                  </div>
                </div>
              </li>
            </ul>
          </section>
        </template>
      </div>
    </section>

    <section v-if="hasTunnels" class="mt-4 shrink-0 px-3">
      <h2 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("terminal.tunnels.title") }}
      </h2>

      <p v-if="tunnelProblem" class="mb-2 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
        <span class="block text-[11px]">{{ tunnelProblem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ tunnelProblem.code }}</span>
        <button
          type="button"
          class="mt-1 rounded-sm text-[10px] text-muted-foreground transition-colors hover:text-foreground"
          @click="terminals.dismissTunnelProblem()"
        >
          {{ t("terminal.tunnels.dismiss") }}
        </button>
      </p>

      <ul v-if="terminals.tunnels.length > 0" class="max-h-44 space-y-0.5 overflow-y-auto">
        <li
          v-for="tunnel in terminals.tunnels"
          :key="tunnel.id"
          class="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-accent/60"
        >
          <Network :size="13" class="shrink-0 text-muted-foreground" />
          <span class="min-w-0 flex-1">
            <span class="block truncate font-mono text-[11px]">
              {{ tunnel.bindHost }}:{{ tunnel.bindPort }} → {{ tunnel.targetHost }}:{{ tunnel.targetPort }}
            </span>
            <span class="block text-[10px] text-muted-foreground">
              {{ t("terminal.tunnels.connections", { count: tunnel.connections }, tunnel.connections) }}
            </span>
          </span>
          <button
            type="button"
            class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
            :title="t('terminal.tunnels.stop')"
            :aria-label="t('terminal.tunnels.stop')"
            @click="terminals.stopTunnel(tunnel.id)"
          >
            <X :size="12" />
          </button>
        </li>
      </ul>

      <ul v-if="terminals.tunnelFailures.length > 0" class="mt-1 max-h-44 space-y-1 overflow-y-auto">
        <li
          v-for="failure in terminals.tunnelFailures"
          :key="tunnelFailureKey(failure)"
          class="rounded-md border border-warning/40 bg-warning/10 px-2 py-1.5"
        >
          <div class="flex items-start gap-1.5">
            <TriangleAlert :size="12" class="mt-0.5 shrink-0 text-warning" />
            <span class="min-w-0 flex-1">
              <span class="block text-[11px]">
                {{ t("terminal.tunnels.failure", { bind: `${failure.bindHost}:${failure.bindPort}` }) }}
              </span>
              <span class="mt-0.5 block font-mono text-[10px] break-all text-muted-foreground">{{
                failure.detail
              }}</span>
            </span>
            <button
              type="button"
              class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
              :title="t('terminal.tunnels.dismiss')"
              :aria-label="t('terminal.tunnels.dismiss')"
              @click="terminals.dismissTunnelFailure(tunnelFailureKey(failure))"
            >
              <X :size="11" />
            </button>
          </div>
        </li>
      </ul>
    </section>

    <section class="mt-4 shrink-0 px-3">
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

    <SessionDialog :open="editorOpen" :session="editing" @update:open="editorOpen = $event" />
  </aside>
</template>
