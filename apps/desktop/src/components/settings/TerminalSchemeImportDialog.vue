<script setup lang="ts">
/**
 * Where a scheme somebody already has is brought in.
 *
 * Pasted text or a picked file: a scheme is a few lines of text either way. The file's picker is
 * the system's own, opened in Rust, and what comes back is the file's name and its text — the
 * paste path needs no boundary at all. Both land in the same reader, and what it recognises is
 * shown as rows somebody can rename before the import is taken: a name the scheme carries, or the
 * file's own, is only a suggestion, and the paste that carries none is still worth bringing in.
 */
import { ArrowLeft, FileUp, X } from "@lucide/vue";
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

import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError } from "@/lib/backend/errors";
import { type ImportedTerminalScheme, parseTerminalSchemeImport } from "@/lib/terminal/importScheme";

const props = defineProps<{ open: boolean }>();

const emit = defineEmits<{
  import: [schemes: readonly ImportedTerminalScheme[]];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

const stage = ref<"input" | "name">("input");
const text = ref("");
const failed = ref(false);
const readProblem = ref<string | null>(null);
const picking = ref(false);
/** What the reader recognised, held as copies so a rename here edits only this import. */
const parsed = ref<ImportedTerminalScheme[]>([]);

function close(): void {
  stage.value = "input";
  text.value = "";
  failed.value = false;
  readProblem.value = null;
  parsed.value = [];
  emit("update:open", false);
}

function updateOpen(value: boolean): void {
  if (!value) {
    close();
    return;
  }
  emit("update:open", true);
}

function recognise(from: string, fallbackName: string): void {
  const result = parseTerminalSchemeImport(from, fallbackName);
  if (result.schemes.length === 0) {
    failed.value = true;
    return;
  }
  parsed.value = result.schemes.map((scheme) => ({ ...scheme }));
  stage.value = "name";
}

function readPasted(): void {
  failed.value = false;
  readProblem.value = null;
  recognise(text.value, t("settings.schemes.import.fallbackName"));
}

/** Reads a file the picker chose, and shows what it holds — the text stays on screen to be edited. */
async function pick(): Promise<void> {
  failed.value = false;
  readProblem.value = null;
  picking.value = true;
  try {
    const file = await backend.settings.pickSchemeImport(t("settings.schemes.import.pickTitle"));
    if (!file) {
      return;
    }
    text.value = file.contents;
    recognise(file.contents, file.name);
  } catch (error) {
    readProblem.value = describeShellError(t, asShellError(error, "scheme_import_pick")).message;
  } finally {
    picking.value = false;
  }
}

function back(): void {
  stage.value = "input";
  parsed.value = [];
}

function drop(index: number): void {
  parsed.value.splice(index, 1);
  if (parsed.value.length === 0) {
    back();
  }
}

function commit(): void {
  const fallback = t("settings.schemes.import.fallbackName");
  const schemes = parsed.value.map((scheme) => ({ ...scheme, name: scheme.name.trim() || fallback }));
  emit("import", schemes);
  close();
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

        <div v-if="stage === 'input'" class="min-h-0 flex-1 space-y-2 overflow-y-auto px-4 py-4 scroll-thin">
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("settings.schemes.import.hint") }}</p>
          <div class="flex items-center justify-between gap-2">
            <button
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="picking"
              @click="pick()"
            >
              <FileUp :size="13" />
              {{ t("settings.schemes.import.pickFile") }}
            </button>
            <p v-if="readProblem" class="min-w-0 flex-1 truncate text-right text-xs text-destructive">
              {{ readProblem }}
            </p>
          </div>
          <textarea
            v-model="text"
            rows="12"
            spellcheck="false"
            class="w-full resize-y rounded-md border border-border bg-background px-3 py-2 font-mono text-xs focus:border-primary focus:outline-none"
            :placeholder="t('settings.schemes.import.placeholder')"
            :aria-label="t('settings.schemes.import.placeholder')"
            @input="failed = false"
          ></textarea>
          <p v-if="failed" class="text-xs text-destructive">{{ t("settings.schemes.import.empty") }}</p>
        </div>

        <div v-else class="min-h-0 flex-1 space-y-2 overflow-y-auto px-4 py-4 scroll-thin">
          <p class="text-xs text-muted-foreground">
            {{ t("settings.schemes.import.found", { count: parsed.length }) }}
          </p>
          <div
            v-for="(scheme, index) in parsed"
            :key="index"
            class="flex items-center gap-3 rounded-md border border-border px-3 py-2"
          >
            <div class="flex h-4 w-28 shrink-0 overflow-hidden rounded-sm border border-border" aria-hidden="true">
              <span
                v-for="(color, slot) in scheme.colors"
                :key="slot"
                class="flex-1"
                :style="{ backgroundColor: color }"
              ></span>
            </div>
            <input
              v-model="scheme.name"
              class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:border-primary focus:outline-none"
              :placeholder="t('settings.schemes.import.namePlaceholder')"
              :aria-label="t('settings.schemes.import.nameLabel')"
            />
            <button
              v-if="parsed.length > 1"
              type="button"
              class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :aria-label="t('settings.schemes.import.dropRow')"
              @click="drop(index)"
            >
              <X :size="13" />
            </button>
          </div>
        </div>

        <footer class="flex justify-end gap-2 border-t border-border px-4 py-3">
          <template v-if="stage === 'input'">
            <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
              {{ t("common.action.cancel") }}
            </DialogClose>
            <button
              type="button"
              class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-60"
              :disabled="text.trim().length === 0"
              @click="readPasted()"
            >
              {{ t("settings.schemes.import.read") }}
            </button>
          </template>
          <template v-else>
            <button
              type="button"
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
              @click="back()"
            >
              <ArrowLeft :size="13" />
              {{ t("settings.schemes.import.back") }}
            </button>
            <button
              type="button"
              class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
              @click="commit()"
            >
              {{ t("settings.schemes.import.commit", { count: parsed.length }) }}
            </button>
          </template>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
