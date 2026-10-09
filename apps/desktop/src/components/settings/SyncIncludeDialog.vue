<script setup lang="ts">
/**
 * The one question a backup asks before it is written: what goes in it?
 *
 * A question rather than a settings page, because the answer is about *this file* and not about
 * this installation — somebody backing up before a reinstall wants everything, and somebody
 * sending a copy to a colleague wants the library and nothing else.
 *
 * The two that are off are off for different reasons, and the dialog says which: the history is a
 * record of what somebody typed, and the trusted host keys decide what another machine accepts
 * before it has ever connected anywhere. The credentials are off until a passphrase is there to
 * seal them, because a backup with no passphrase carries none of them — the switch follows the
 * field on the panel behind this dialog rather than pretending otherwise.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { LoaderCircle } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import type { SyncContents, SyncSelection } from "@/types/sync";

const props = defineProps<{
  open: boolean;
  /** While the write is running: the dialog stays up and nothing can be pressed twice. */
  busy: boolean;
  /** What this machine could put in, for the sentence that says what always goes. */
  available: SyncContents | null;
  /** Whether a passphrase was typed, which is what decides if credentials can travel at all. */
  canSeal: boolean;
}>();

const emit = defineEmits<{ confirm: [selection: SyncSelection]; "update:open": [value: boolean] }>();

const { t } = useI18n();

const settings = ref(true);
const history = ref(false);
const hostKeys = ref(false);
const credentials = ref(false);

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    // Every opening starts from nobody's surprise: the settings travel, the diary and the trust
    // list do not, and the credentials follow the passphrase that is actually there.
    settings.value = true;
    history.value = false;
    hostKeys.value = false;
    credentials.value = false;
  },
  { immediate: true },
);

/** What every backup carries, said as the counts this machine has. */
const always = computed(() => {
  const counts = props.available;
  if (!counts) {
    return "";
  }
  const parts: string[] = [];
  const add = (key: string, count: number): void => {
    if (count > 0) {
      parts.push(t(`sync.${key}`, { count }, count));
    }
  };
  add("sessions", counts.sessions);
  add("groups", counts.groups);
  add("snippets", counts.snippets);
  add("words", counts.words);

  return parts.join(", ");
});

function confirm(): void {
  emit("confirm", {
    credentials: credentials.value && props.canSeal,
    history: history.value,
    knownHosts: hostKeys.value,
    settings: settings.value,
  });
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="px-4 pt-4">
          <DialogTitle class="text-sm font-medium">{{ t("sync.includeTitle") }}</DialogTitle>
          <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
            {{ t("sync.includeMessage") }}
            <span v-if="always" class="mt-1 block">{{ t("sync.includeAlways", { parts: always }) }}</span>
          </DialogDescription>

          <div class="mt-3 space-y-2">
            <label class="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
              <input v-model="settings" type="checkbox" class="mt-0.5 size-3.5 shrink-0 accent-primary" />
              <span class="min-w-0 block text-xs leading-relaxed">{{ t("sync.includeSettings") }}</span>
            </label>

            <label class="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
              <input v-model="history" type="checkbox" class="mt-0.5 size-3.5 shrink-0 accent-primary" />
              <span class="min-w-0">
                <span class="block text-xs">{{ t("sync.includeHistory") }}</span>
                <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
                  {{ t("sync.includeHistoryHint") }}
                </span>
              </span>
            </label>

            <label class="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
              <input v-model="hostKeys" type="checkbox" class="mt-0.5 size-3.5 shrink-0 accent-primary" />
              <span class="min-w-0">
                <span class="block text-xs">{{ t("sync.includeHosts") }}</span>
                <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
                  {{ t("sync.includeHostsHint") }}
                </span>
              </span>
            </label>

            <label class="flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
              <input
                v-model="credentials"
                type="checkbox"
                class="mt-0.5 size-3.5 shrink-0 accent-primary"
                :disabled="!props.canSeal"
              />
              <span class="min-w-0">
                <span class="block text-xs" :class="props.canSeal ? '' : 'text-muted-foreground'">
                  {{ t("sync.includeCredentials") }}
                </span>
                <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
                  {{ t("sync.includeCredentialsHint") }}
                </span>
              </span>
            </label>
          </div>
        </div>

        <footer
          class="mt-4 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="emit('update:open', false)"
          >
            {{ t("common.action.cancel") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="confirm()"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ t("sync.includeConfirm") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
