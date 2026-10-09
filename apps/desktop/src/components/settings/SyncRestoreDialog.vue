<script setup lang="ts">
/**
 * The look before a restore: what the file holds, and the one thing that decides what happens to
 * what is already here.
 *
 * The replace switch is off every time, and deliberately not remembered: a restore that quietly
 * wrote over a library because somebody ticked a box a week ago is the worst kind of surprise this
 * feature could produce. Off, an item whose id is already here arrives as a second copy — which is
 * recoverable by deleting it, and the other way round is not.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { LoaderCircle } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import type { ReadBackup } from "@/stores/sync";

const props = defineProps<{
  open: boolean;
  busy: boolean;
  /** The file that was read, with what the shell said is inside it. */
  read: ReadBackup | null;
}>();

const emit = defineEmits<{ confirm: [replace: boolean]; "update:open": [value: boolean] }>();

const { t } = useI18n();

const replace = ref(false);

watch(
  () => props.open,
  (open) => {
    if (open) {
      replace.value = false;
    }
  },
  { immediate: true },
);

/** What the file holds, as counts — the same words the receipt uses, from the same keys. */
const holds = computed(() => {
  const contents = props.read?.catalog.contents;
  if (!contents) {
    return "";
  }
  const parts: string[] = [];
  const add = (key: string, count: number): void => {
    if (count > 0) {
      parts.push(t(`sync.${key}`, { count }, count));
    }
  };
  add("sessions", contents.sessions);
  add("groups", contents.groups);
  add("snippets", contents.snippets);
  add("words", contents.words);
  add("history", contents.history);
  add("hosts", contents.knownHosts);
  add("settings", contents.settings);
  add("secrets", contents.secrets);
  add("keys", contents.keys);

  return parts.join(", ");
});
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="px-4 pt-4">
          <DialogTitle class="text-sm font-medium">{{ t("sync.restoreTitle") }}</DialogTitle>
          <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
            {{ t("sync.restoreMessage", { file: props.read?.fileName ?? "" }) }}
          </DialogDescription>

          <ul class="mt-3 max-h-40 space-y-0.5 overflow-y-auto rounded-md border border-border bg-muted/40 px-3 py-2">
            <li class="text-[11px] leading-relaxed text-muted-foreground">{{ holds }}</li>
          </ul>

          <p v-if="props.read?.catalog.sealed" class="mt-2 text-[11px] leading-relaxed text-muted-foreground">
            {{ t("sync.restoreSealed") }}
          </p>

          <label class="mt-3 flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
            <input v-model="replace" type="checkbox" class="mt-0.5 size-3.5 shrink-0 accent-primary" />
            <span class="min-w-0">
              <span class="block text-xs">{{ t("sync.replace") }}</span>
              <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
                {{ t("sync.replaceHint") }}
              </span>
            </span>
          </label>
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
            @click="emit('confirm', replace)"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ t("sync.restoreConfirm") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
