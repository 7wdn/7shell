<script setup lang="ts">
/**
 * The file browser's quick jumps: named remote directories, one click from the path bar.
 *
 * The list is one for every connection — a bookmark is a place somebody goes back to, and the same
 * `/var/log` is usually the same place on every server they run — and it is written through the
 * preferences store the moment it changes: this popover is not the settings page, so there is no
 * later Save that would carry the list to the file.
 *
 * Removing is immediate rather than asked about: a bookmark is one line somebody can re-make in a
 * few seconds, and a question before each removal would be noise in the way of the list itself.
 */
import { Bookmark, BookmarkPlus, Copy, Crosshair, Folder, Pencil, Trash2 } from "@lucide/vue";
import {
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger,
} from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import {
  FILE_BOOKMARK_LABEL_MAX,
  FILE_BOOKMARK_LIMIT,
  FILE_BOOKMARK_PATH_MAX,
  type FileBookmarkError,
  bookmarkLabelForPath,
  validateFileBookmark,
} from "@/lib/files/bookmarks";
import { usePreferencesStore } from "@/stores/preferences";
import type { FileBookmark } from "@/types/app";

const props = defineProps<{ path: string }>();
const emit = defineEmits<{ jump: [path: string] }>();

const { t } = useI18n();
const preferences = usePreferencesStore();
const { toast } = useToast();

/** The popover, and the draft the add/edit dialog is about. Exactly one of them is open. */
const open = ref(false);
const draft = ref<BookmarkDraft | null>(null);

interface BookmarkDraft {
  /** The entry being rewritten, or null for one that does not exist yet. */
  readonly id: string | null;
  readonly label: string;
  readonly path: string;
}

const bookmarks = computed(() => preferences.fileBookmarks);
const atLimit = computed(() => bookmarks.value.length >= FILE_BOOKMARK_LIMIT);

/** The first rule the draft breaks, drawn under the fields while it is being typed. */
const draftError = computed<FileBookmarkError | null>(() => {
  const current = draft.value;
  if (!current) return null;
  return validateFileBookmark(current, bookmarks.value, { id: current.id ?? undefined });
});

/** The dialog's fields, as writable views of the draft so a null draft has nothing to write to. */
const draftLabel = computed({
  get: () => draft.value?.label ?? "",
  set: (value: string) => {
    if (draft.value) draft.value = { ...draft.value, label: value };
  },
});

const draftPath = computed({
  get: () => draft.value?.path ?? "",
  set: (value: string) => {
    if (draft.value) draft.value = { ...draft.value, path: value };
  },
});

/** Every action at the end of a row, drawn the same way. */
const ROW_ACTION =
  "grid size-6 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-background hover:text-foreground";

/** Reads an error code as the sentence the popover shows, with the number the rule mentions. */
function errorText(error: FileBookmarkError): string {
  if (error === "labelTooLong") return t("files.bookmark.error.labelTooLong", { limit: FILE_BOOKMARK_LABEL_MAX });
  if (error === "pathTooLong") return t("files.bookmark.error.pathTooLong", { limit: FILE_BOOKMARK_PATH_MAX });
  if (error === "limitReached") return t("files.bookmark.error.limitReached", { limit: FILE_BOOKMARK_LIMIT });
  return t(`files.bookmark.error.${error}`);
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** Starts a draft for the directory on screen, named after its last segment. */
function startAdd(): void {
  if (atLimit.value) {
    toast(t("files.bookmark.error.limitReached", { limit: FILE_BOOKMARK_LIMIT }), "refusal");
    return;
  }
  open.value = false;
  draft.value = {
    id: null,
    label: bookmarkLabelForPath(props.path),
    path: props.path,
  };
}

function startEdit(bookmark: FileBookmark): void {
  open.value = false;
  draft.value = { id: bookmark.id, label: bookmark.label, path: bookmark.path };
}

/**
 * Saves the draft, or says which rule it breaks.
 *
 * The choice holds for this run even when the file cannot be written — the store applies before it
 * persists — so the failure is said out loud and the dialog closes rather than trapping somebody in
 * a field whose value has already been taken.
 */
async function saveDraft(): Promise<void> {
  const current = draft.value;
  const failure = draftError.value;
  if (!current) return;
  if (failure) {
    toast(errorText(failure), "refusal");
    return;
  }
  const label = current.label.trim();
  const input = { label, path: current.path };
  const problem = current.id
    ? await preferences.updateFileBookmark(current.id, input)
    : await preferences.addFileBookmark(input);
  if (problem) {
    toast(errorText(problem), "refusal");
    return;
  }
  draft.value = null;
  if (preferences.problem) {
    toast(describeShellError(translate, preferences.problem).message, "failure");
    return;
  }
  toast(current.id ? t("files.bookmark.updated", { label }) : t("files.bookmark.added", { label }), "confirmation");
}

function jumpTo(path: string): void {
  open.value = false;
  emit("jump", path);
}

async function copyPath(bookmark: FileBookmark): Promise<void> {
  try {
    await navigator.clipboard.writeText(bookmark.path);
    toast(t("files.bookmark.copied"), "receipt");
  } catch {
    toast(t("files.bookmark.copyFailed"), "failure");
  }
}

async function remove(bookmark: FileBookmark): Promise<void> {
  await preferences.removeFileBookmark(bookmark.id);
  toast(t("files.bookmark.removed", { label: bookmark.label }), "receipt");
}
</script>

<template>
  <PopoverRoot :open="open" @update:open="(value: boolean) => (open = value)">
    <PopoverTrigger as-child>
      <button
        type="button"
        data-bookmark-menu
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('files.bookmark.title')"
        :aria-label="t('files.bookmark.title')"
        :aria-expanded="open"
      >
        <Bookmark :size="13" />
      </button>
    </PopoverTrigger>

    <PopoverPortal>
      <PopoverContent
        side="bottom"
        align="end"
        :side-offset="6"
        :collision-padding="8"
        data-file-bookmarks
        class="z-50 flex max-h-[min(24rem,60vh)] w-80 flex-col rounded-lg border border-border bg-card p-1.5 shadow-xl"
      >
        <div class="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
          <span class="text-xs font-medium">{{ t("files.bookmark.title") }}</span>
          <span class="rounded-full bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">{{
            bookmarks.length
          }}</span>
          <span class="flex-1" />
          <button
            type="button"
            data-bookmark-add
            class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            :disabled="atLimit"
            :title="
              atLimit ? t('files.bookmark.error.limitReached', { limit: FILE_BOOKMARK_LIMIT }) : t('files.bookmark.add')
            "
            :aria-label="t('files.bookmark.add')"
            @click="startAdd"
          >
            <BookmarkPlus :size="13" />
          </button>
        </div>

        <div class="min-h-0 flex-1 overflow-y-auto scroll-thin">
          <div
            v-for="bookmark in bookmarks"
            :key="bookmark.id"
            data-bookmark-row
            class="flex items-center gap-1 rounded-md px-1.5 py-1 transition-colors hover:bg-accent"
          >
            <Folder :size="13" class="shrink-0 text-muted-foreground" />
            <button
              type="button"
              class="min-w-0 flex-1 text-left"
              :title="bookmark.path"
              :aria-label="`${bookmark.label} · ${bookmark.path}`"
              @click="jumpTo(bookmark.path)"
            >
              <span class="block truncate text-xs">{{ bookmark.label }}</span>
              <span class="block truncate font-mono text-[10px] text-muted-foreground">{{ bookmark.path }}</span>
            </button>
            <button
              type="button"
              data-bookmark-action="jump"
              :class="ROW_ACTION"
              :title="t('files.bookmark.jump')"
              :aria-label="t('files.bookmark.jump')"
              @click="jumpTo(bookmark.path)"
            >
              <Crosshair :size="12" />
            </button>
            <button
              type="button"
              data-bookmark-action="copy"
              :class="ROW_ACTION"
              :title="t('files.bookmark.copy')"
              :aria-label="t('files.bookmark.copy')"
              @click="copyPath(bookmark)"
            >
              <Copy :size="12" />
            </button>
            <button
              type="button"
              data-bookmark-action="edit"
              :class="ROW_ACTION"
              :title="t('files.bookmark.edit')"
              :aria-label="t('files.bookmark.edit')"
              @click="startEdit(bookmark)"
            >
              <Pencil :size="12" />
            </button>
            <button
              type="button"
              data-bookmark-action="remove"
              :class="ROW_ACTION"
              :title="t('files.bookmark.remove')"
              :aria-label="t('files.bookmark.remove')"
              @click="remove(bookmark)"
            >
              <Trash2 :size="12" />
            </button>
          </div>

          <p v-if="bookmarks.length === 0" class="px-2 py-6 text-center text-[11px] text-muted-foreground">
            {{ t("files.bookmark.empty") }}
          </p>
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>

  <DialogRoot
    :open="draft !== null"
    @update:open="
      (value: boolean) => {
        if (!value) draft = null;
      }
    "
  >
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        data-bookmark-dialog
        class="fixed top-1/2 left-1/2 z-50 w-[min(26rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl select-none focus:outline-none"
      >
        <div class="px-4 pt-4">
          <DialogTitle class="text-sm font-medium">
            {{ draft?.id ? t("files.bookmark.editTitle") : t("files.bookmark.addTitle") }}
          </DialogTitle>
          <DialogDescription class="mt-1 text-[11px] text-muted-foreground">
            {{ t("files.bookmark.dialogHint") }}
          </DialogDescription>

          <label class="mt-3 block">
            <span class="mb-1 block text-[11px] text-muted-foreground">{{ t("files.bookmark.name") }}</span>
            <input
              v-model="draftLabel"
              type="text"
              spellcheck="false"
              :maxlength="FILE_BOOKMARK_LABEL_MAX"
              class="w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:outline-none"
              :aria-label="t('files.bookmark.name')"
            />
          </label>

          <label class="mt-2 block">
            <span class="mb-1 block text-[11px] text-muted-foreground">{{ t("files.bookmark.path") }}</span>
            <input
              v-model="draftPath"
              type="text"
              spellcheck="false"
              :maxlength="FILE_BOOKMARK_PATH_MAX"
              class="w-full rounded-md border border-border bg-background px-2 py-1.5 font-mono text-[11px] focus:outline-none"
              :aria-label="t('files.bookmark.path')"
              @keydown.enter.prevent="saveDraft"
            />
          </label>

          <p v-if="draftError" role="alert" class="mt-2 text-[11px] text-destructive">{{ errorText(draftError) }}</p>
        </div>

        <footer class="mt-4 flex justify-end gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3">
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
            @click="draft = null"
          >
            {{ t("files.cancel") }}
          </button>
          <button
            type="button"
            data-bookmark-save
            class="rounded-md border border-primary/40 bg-primary/10 px-3 py-1.5 text-xs text-primary transition-colors hover:bg-primary/20 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="draftError !== null"
            @click="saveDraft"
          >
            {{ t("files.bookmark.save") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
