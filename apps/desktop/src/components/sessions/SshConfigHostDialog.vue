<script setup lang="ts">
/**
 * The hosts this machine's OpenSSH configuration names.
 *
 * Choosing one does not copy anything: the alias is what the session will store, and the values
 * shown beside it are this machine's current answer. A host whose configuration uses a directive
 * the connection cannot honour is listed with the reason and cannot be chosen — a session that
 * refuses on its first connection is worse than a row that says why now.
 */
import { LoaderCircle, Search, Terminal } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import type { SshConfigHost } from "@/types/session";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{
  "update:open": [value: boolean];
  select: [host: SshConfigHost];
}>();

const { t } = useI18n();

const hosts = ref<SshConfigHost[]>([]);
const busy = ref(false);
const failure = ref<ShellError | null>(null);
/** Folded into the alias, the address and the account, because a person recognises any of them. */
const filter = ref("");
const field = ref<HTMLInputElement | null>(null);

const visibleHosts = computed(() => {
  const needle = filter.value.trim().toLowerCase();
  if (needle === "") {
    return hosts.value;
  }
  return hosts.value.filter((host) =>
    [host.alias, host.host, host.user ?? ""].some((value) => value.toLowerCase().includes(needle)),
  );
});

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = (): string => (failure.value ? describeShellError(translate, failure.value).message : "");

async function load(): Promise<void> {
  busy.value = true;
  failure.value = null;
  try {
    hosts.value = await backend.sessions.sshConfigHosts();
  } catch (error) {
    failure.value = asShellError(error, "ssh_config_hosts");
  } finally {
    busy.value = false;
  }
}

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      return;
    }
    filter.value = "";
    await load();
    // The filter appears with the hosts it narrows, so focus is taken once the list is drawn.
    await nextTick();
    field.value?.focus();
  },
);

function choose(host: SshConfigHost): void {
  if (host.unsupported.length > 0) {
    return;
  }
  emit("select", host);
  emit("update:open", false);
}

function endpoint(host: SshConfigHost): string {
  return `${host.host}:${host.port ?? 22}`;
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(34rem,calc(100vh-4rem))] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="px-4 pt-4">
          <div class="flex items-start gap-2.5">
            <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background">
              <Terminal :size="14" class="text-muted-foreground" />
            </span>
            <div class="min-w-0">
              <DialogTitle class="text-sm font-medium">{{ t("sessions.editor.openssh.dialogTitle") }}</DialogTitle>
              <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t("sessions.editor.openssh.dialogDescription") }}
              </DialogDescription>
            </div>
          </div>
        </div>

        <div v-if="!busy && !failure && hosts.length > 0" class="relative mx-4 mt-3">
          <Search
            :size="14"
            class="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground"
          />
          <input
            ref="field"
            v-model="filter"
            data-ssh-config-filter
            type="text"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            class="h-8 w-full rounded-md border border-border bg-background pr-3 pl-8 text-xs select-text focus:ring-1 focus:ring-ring focus:outline-none"
            :placeholder="t('sessions.editor.openssh.filterPlaceholder')"
            :aria-label="t('sessions.editor.openssh.filterPlaceholder')"
          />
        </div>

        <div class="mt-3 min-h-0 flex-1 overflow-auto px-4">
          <p v-if="busy" class="flex items-center gap-2 py-4 text-xs text-muted-foreground">
            <LoaderCircle :size="13" class="animate-spin" />
            {{ t("sessions.editor.openssh.loading") }}
          </p>

          <p
            v-else-if="failure"
            role="alert"
            class="my-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ problem() }}</span>
            <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ failure.code }}</span>
          </p>

          <p v-else-if="hosts.length === 0" class="py-4 text-xs text-muted-foreground">
            {{ t("sessions.editor.openssh.empty") }}
          </p>

          <p v-else-if="visibleHosts.length === 0" class="py-4 text-xs text-muted-foreground">
            {{ t("sessions.editor.openssh.noMatch") }}
          </p>

          <ul v-else class="space-y-1 pb-2">
            <li v-for="host in visibleHosts" :key="host.alias">
              <button
                type="button"
                data-ssh-config-host
                class="flex w-full items-start gap-3 rounded-md border border-border px-3 py-2 text-left transition-colors enabled:hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
                :disabled="host.unsupported.length > 0"
                @click="choose(host)"
              >
                <span
                  class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-sm bg-muted font-mono text-[10px] text-muted-foreground"
                >
                  {{ host.alias.slice(0, 2) }}
                </span>
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-xs font-medium">{{ host.alias }}</span>
                  <span class="block truncate font-mono text-[11px] text-muted-foreground">{{ endpoint(host) }}</span>
                  <span v-if="host.user" class="block truncate font-mono text-[11px] text-muted-foreground">
                    {{ host.user }}
                  </span>
                  <span v-if="host.proxyJump" class="block truncate font-mono text-[11px] text-muted-foreground">
                    {{ t("sessions.editor.openssh.viaProxy", { route: host.proxyJump }) }}
                  </span>
                  <span v-if="host.unsupported.length > 0" class="mt-1 block text-[11px] text-warning">
                    {{ t("sessions.editor.openssh.unsupported", { directives: host.unsupported.join(", ") }) }}
                  </span>
                </span>
              </button>
            </li>
          </ul>
        </div>

        <footer class="flex justify-end border-t border-border px-4 py-3">
          <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
            {{ t("sessions.editor.openssh.close") }}
          </DialogClose>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
