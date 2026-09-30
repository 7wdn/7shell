<script setup lang="ts">
/**
 * One remote file, edited as plain text.
 *
 * It loads through `files.read_text` and saves through `files.write_text`, and shows whatever the
 * backend refuses with. A file that is not text this build can show, or one too large to open, comes
 * back as an envelope of its own — guessing at those limits on this side would put the same rule in
 * two places and let the two disagree.
 *
 * A plain editor on purpose: no syntax highlighting yet, and the dialog says so rather than letting
 * a monospace textarea suggest more than it does.
 */
import { LoaderCircle, Save } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { baseName } from "@/lib/files/format";

const props = defineProps<{
  open: boolean;
  handle: string;
  /** The full path, so the editor never builds one from a name. */
  path: string;
}>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const text = ref("");
const loading = ref(false);
const saving = ref(false);
const failure = ref<ShellError | null>(null);

const title = computed(() => t("files.editor.title", { name: baseName(props.path) }));
const problem = computed(() => (failure.value ? describeShellError(translate, failure.value) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

watch(
  () => [props.open, props.handle, props.path] as const,
  async ([open]) => {
    if (!open) {
      return;
    }
    text.value = "";
    failure.value = null;
    loading.value = true;
    try {
      text.value = await backend.files.readText(props.handle, props.path);
    } catch (error) {
      failure.value = asShellError(error, "files_read_text");
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

async function save(): Promise<void> {
  saving.value = true;
  failure.value = null;
  try {
    await backend.files.writeText(props.handle, props.path, text.value);
    emit("update:open", false);
  } catch (error) {
    failure.value = asShellError(error, "files_write_text");
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex w-[min(52rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ title }}</DialogTitle>
          <DialogDescription class="mt-0.5 text-[11px] text-muted-foreground">
            {{ t("files.editor.description") }}
          </DialogDescription>
        </header>

        <form class="flex min-h-0 flex-col gap-3 px-4 py-4" @submit.prevent="save()">
          <p class="shrink-0 font-mono text-[10px] break-all text-muted-foreground">{{ props.path }}</p>

          <div class="relative">
            <textarea
              v-model="text"
              spellcheck="false"
              class="h-[50vh] w-full resize-none rounded-md border border-border bg-background px-3 py-2 font-mono text-xs focus:outline-none"
              :aria-label="t('files.editor.description')"
            ></textarea>
            <p
              v-if="loading"
              class="absolute inset-0 grid place-items-center bg-background/70 text-xs text-muted-foreground"
            >
              <span class="flex items-center gap-2">
                <LoaderCircle :size="13" class="animate-spin" />
                {{ t("files.editor.loading") }}
              </span>
            </p>
          </div>

          <p class="shrink-0 text-[11px] text-muted-foreground">{{ t("files.editor.note") }}</p>

          <p
            v-if="problem"
            class="shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ problem.message }}</span>
            <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
          </p>

          <div class="flex shrink-0 justify-end gap-2 border-t border-border pt-4">
            <button
              type="button"
              class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
              @click="emit('update:open', false)"
            >
              {{ t("files.cancel") }}
            </button>
            <button
              type="submit"
              class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
              :disabled="saving || loading"
            >
              <Save :size="12" />
              {{ t("files.editor.save") }}
            </button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
