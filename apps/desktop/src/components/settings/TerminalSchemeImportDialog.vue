<script setup lang="ts">
/**
 * Where a scheme somebody already has is brought in.
 *
 * Pasted text rather than a file picker: a scheme is a few lines, every terminal's export is text,
 * and opening the system's picker would mean carrying a file through the shell for something this
 * window can read itself. The dialog parses, says whether the text was recognised, and closes when
 * the schemes are taken.
 */
import { X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { ref } from "vue";
import { useI18n } from "vue-i18n";

import { type ImportedTerminalScheme, parseTerminalSchemeImport } from "@/lib/terminal/importScheme";

const props = defineProps<{ open: boolean }>();

const emit = defineEmits<{
  import: [schemes: readonly ImportedTerminalScheme[]];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

const text = ref("");
const failed = ref(false);

function close(): void {
  text.value = "";
  failed.value = false;
  emit("update:open", false);
}

function submit(): void {
  const parsed = parseTerminalSchemeImport(text.value);
  if (parsed.schemes.length === 0) {
    failed.value = true;
    return;
  }
  emit("import", parsed.schemes);
  close();
}

function updateOpen(value: boolean): void {
  if (!value) {
    close();
    return;
  }
  emit("update:open", true);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="updateOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(40rem,calc(100vh-3rem))] w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ t("settings.schemes.import.title") }}</DialogTitle>
          <DialogDescription class="sr-only">{{ t("settings.schemes.import.hint") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('settings.schemes.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <div class="min-h-0 flex-1 space-y-2 overflow-y-auto px-4 py-4 scroll-thin">
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("settings.schemes.import.hint") }}</p>
          <textarea
            v-model="text"
            rows="14"
            spellcheck="false"
            class="w-full resize-y rounded-md border border-border bg-background px-3 py-2 font-mono text-xs focus:border-primary focus:outline-none"
            :placeholder="t('settings.schemes.import.placeholder')"
            :aria-label="t('settings.schemes.import.placeholder')"
            @input="failed = false"
          ></textarea>
          <p v-if="failed" class="text-xs text-destructive">{{ t("settings.schemes.import.empty") }}</p>
        </div>

        <footer class="flex justify-end gap-2 border-t border-border px-4 py-3">
          <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
            {{ t("common.action.cancel") }}
          </DialogClose>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
            @click="submit()"
          >
            {{ t("settings.schemes.import.action") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
