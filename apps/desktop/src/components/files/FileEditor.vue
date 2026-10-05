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
 *
 * Closing asks when something was typed and not saved. Every way out goes through the same question
 * — the Cancel button, Escape, a click on the overlay — because the way out is not what loses the
 * work; the unsaved buffer is. Escape in particular used to close the editor outright, which made
 * the key that people press to back out of a dialog the one that threw their edits away.
 */
import { LoaderCircle, Save } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { baseName } from "@/lib/files/format";
import { useFilesStore } from "@/stores/files";

const props = defineProps<{
  open: boolean;
  handle: string;
  /** The full path, so the editor never builds one from a name. */
  path: string;
}>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const files = useFilesStore();
const text = ref("");
/** What was read, or last written: the buffer is "dirty" when it no longer matches this. */
const original = ref("");
const loading = ref(false);
const saving = ref(false);
const failure = ref<ShellError | null>(null);
/** Whether the question about unsaved changes is on screen. */
const asking = ref(false);

const title = computed(() => t("files.editor.title", { name: baseName(props.path) }));
const problem = computed(() => (failure.value ? describeShellError(translate, failure.value) : null));
const dirty = computed(() => !loading.value && text.value !== original.value);

/**
 * What this editor is called in the list of editors holding unsaved text.
 *
 * The shell reads that list when the window is asked to close, so a quit that would throw this away
 * asks first — a dialog is not something the frame can look inside, and the buffer is not something
 * its own close path can vouch for.
 */
const unsavedKey = computed(() => `${props.handle}:${props.path}`);
watch(dirty, (value) => files.setUnsaved(unsavedKey.value, value), { immediate: true });
onBeforeUnmount(() => files.setUnsaved(unsavedKey.value, false));

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
    original.value = "";
    failure.value = null;
    asking.value = false;
    loading.value = true;
    try {
      text.value = await backend.files.readText(props.handle, props.path);
      original.value = text.value;
    } catch (error) {
      failure.value = asShellError(error, "files_read_text");
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

/** True when the file was written, so a close that waited on it knows whether it may go. */
async function save(): Promise<boolean> {
  saving.value = true;
  failure.value = null;
  try {
    await backend.files.writeText(props.handle, props.path, text.value);
    original.value = text.value;
    return true;
  } catch (error) {
    failure.value = asShellError(error, "files_write_text");
    return false;
  } finally {
    saving.value = false;
  }
}

/** The one way out: ask if there is something to lose, and leave if there is not. */
function requestClose(): void {
  if (dirty.value) {
    asking.value = true;
    return;
  }
  emit("update:open", false);
}

/** "Discard": the question is the last thing standing between the buffer and the bin. */
function discardAndClose(): void {
  asking.value = false;
  emit("update:open", false);
}

/** "Save and close": a write that failed keeps the editor, with the reason in it. */
async function saveAndClose(): Promise<void> {
  if (await save()) {
    asking.value = false;
    emit("update:open", false);
    return;
  }
  // The question stands aside so the failure it caused is the thing on screen.
  asking.value = false;
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex w-[min(52rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @escape-key-down.prevent="requestClose"
        @interact-outside.prevent="requestClose"
      >
        <header class="border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ title }}</DialogTitle>
          <DialogDescription class="mt-0.5 text-[11px] text-muted-foreground">
            {{ t("files.editor.description") }}
          </DialogDescription>
        </header>

        <form class="flex min-h-0 flex-col gap-3 px-4 py-4" @submit.prevent="saveAndClose()">
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
              @click="requestClose()"
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

  <!--
    The question about what was typed and not saved. It is drawn over the editor rather than in it:
    the editor is the thing that has to stay put while somebody decides about it, and its own
    Escape has already been taken over by this.
  -->
  <ConfirmDialog
    v-if="asking"
    :open="true"
    :title="t('files.editor.unsavedTitle')"
    :message="t('files.editor.unsavedMessage', { name: baseName(props.path) })"
    :cancel-label="t('files.editor.unsavedKeep')"
    :discard-label="t('files.editor.unsavedDiscard')"
    :confirm-label="t('files.editor.unsavedSave')"
    :loading="saving"
    @discard="discardAndClose()"
    @confirm="saveAndClose()"
    @update:open="asking = false"
  />
</template>
