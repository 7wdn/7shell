<script setup lang="ts">
/**
 * One remote directory, drawn as a panel: where it is, what is in it, and what can be done with
 * each thing in it.
 *
 * Entries are drawn in the order they arrive. The backend sorts directories first and then names,
 * and sorting again here would be a second answer to the same question — one that would drift the
 * first time either side changed its mind.
 *
 * Nothing in this panel names a local path, and it could not: a download and an upload ask where to
 * write or what to read inside Rust, and the answer never comes back to this side. That is why the
 * two actions only ever hand over a title and a suggested name.
 *
 * The backend re-reads the directory after every change and after an upload lands, so what is on
 * screen is the server's answer rather than this component's guess at what a change did.
 */
import {
  ArrowUp,
  ChevronRight,
  Download,
  File,
  FileSymlink,
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  HardDrive,
  LoaderCircle,
  Pencil,
  Trash2,
  Upload,
  X,
} from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import DirectoryTree from "@/components/files/DirectoryTree.vue";
import FileEditor from "@/components/files/FileEditor.vue";
import TransferList from "@/components/files/TransferList.vue";
import { describeShellError } from "@/i18n/errors";
import { formatModified, formatSize, joinPath, parentPath } from "@/lib/files/format";
import { cn } from "@/lib/utils";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import type { SftpEntry } from "@/types/session";

const props = defineProps<{ handle: string }>();

const preferences = usePreferencesStore();

/**
 * Whether the tree is shown beside the listing.
 *
 * The default is the tree, because a remote file system is easiest to move around in when you can
 * see where you are; the single list is there for a narrow window. The choice is a preference, so
 * it applies to every browser that is open and survives a restart.
 */
const twoPanes = computed(() => preferences.filesLayout === "tree");

/** Goes where the tree asked, through the same path the listing uses. */
function goTo(path: string): void {
  void files.navigate(props.handle, path);
}

const { t } = useI18n();
const files = useFilesStore();

const browser = computed(() => files.browser(props.handle));
const problem = computed(() => (browser.value?.failure ? describeShellError(translate, browser.value.failure) : null));
const parent = computed(() => (browser.value ? parentPath(browser.value.path) : null));
const crumbs = computed(() => (browser.value ? crumbsOf(browser.value.path) : []));

/** The row that was clicked last. Nothing is done to it until an action says so. */
const selected = ref<string | null>(null);
const typedPath = ref("");
const typingNewFolder = ref(false);
const newFolderName = ref("");
/** The path being renamed, so a server refusal can leave the typed name in place. */
const renaming = ref<string | null>(null);
const renameText = ref("");
const confirming = ref<SftpEntry | null>(null);
const editing = ref<SftpEntry | null>(null);
const busy = ref(false);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** The breadcrumbs: the root, then one segment per step, each of which navigates. */
function crumbsOf(path: string): { name: string; path: string }[] {
  if (!path.startsWith("/")) {
    return [{ name: path, path }];
  }
  const built = [{ name: "/", path: "/" }];
  let current = "";
  for (const segment of path.split("/")) {
    if (segment === "") {
      continue;
    }
    current += `/${segment}`;
    built.push({ name: segment, path: current });
  }
  return built;
}

function entryIcon(entry: SftpEntry) {
  switch (entry.kind) {
    case "directory":
      return Folder;
    case "file":
      return File;
    case "symlink":
      return FileSymlink;
    case "other":
      return HardDrive;
  }
}

/** A directory's size is the server's business, not something a listing can say. */
function sizeOf(entry: SftpEntry): string {
  return entry.kind === "directory" ? "" : formatSize(entry.size);
}

function modifiedOf(entry: SftpEntry): string {
  return entry.modifiedAt === undefined ? "—" : formatModified(entry.modifiedAt);
}

/** Files and links can be downloaded or edited; a socket or a device cannot. */
function isReadable(entry: SftpEntry): boolean {
  return entry.kind === "file" || entry.kind === "symlink";
}

watch(
  () => browser.value?.path,
  (path) => {
    typedPath.value = path ?? "";
  },
  { immediate: true },
);

function enter(entry: SftpEntry): void {
  selected.value = entry.path;
  if (entry.kind === "directory") {
    go(entry.path);
  }
}

function go(path: string): void {
  selected.value = null;
  void files.navigate(props.handle, path);
}

function submitTypedPath(): void {
  const path = typedPath.value.trim();
  if (path !== "") {
    go(path);
  }
}

function startNewFolder(): void {
  cancelPrompt();
  newFolderName.value = "";
  typingNewFolder.value = true;
}

function startRename(entry: SftpEntry): void {
  cancelPrompt();
  renaming.value = entry.path;
  renameText.value = entry.name;
}

function startRemove(entry: SftpEntry): void {
  cancelPrompt();
  confirming.value = entry;
}

function cancelPrompt(): void {
  typingNewFolder.value = false;
  renaming.value = null;
  confirming.value = null;
}

async function createFolder(): Promise<void> {
  const current = browser.value;
  const name = newFolderName.value.trim();
  if (!current || name === "") {
    return;
  }
  busy.value = true;
  const worked = await files.createFolder(props.handle, joinPath(current.path, name));
  busy.value = false;
  if (worked) {
    typingNewFolder.value = false;
    newFolderName.value = "";
  }
}

async function submitRename(): Promise<void> {
  const current = browser.value;
  const from = renaming.value;
  const name = renameText.value.trim();
  if (!current || from === null || name === "") {
    return;
  }
  busy.value = true;
  const worked = await files.rename(props.handle, from, joinPath(current.path, name));
  busy.value = false;
  if (worked) {
    renaming.value = null;
  }
}

async function remove(entry: SftpEntry): Promise<void> {
  busy.value = true;
  const worked = await files.remove(props.handle, entry.path, entry.kind);
  busy.value = false;
  if (worked) {
    confirming.value = null;
    if (selected.value === entry.path) {
      selected.value = null;
    }
  }
}

function download(entry: SftpEntry): void {
  void files.download(props.handle, entry.path, t("files.downloadTitle"), entry.name);
}

function upload(): void {
  const current = browser.value;
  if (current) {
    void files.upload(props.handle, current.path, t("files.uploadTitle"));
  }
}

function closeEditor(open: boolean): void {
  if (!open) {
    editing.value = null;
  }
}
</script>

<template>
  <div v-if="browser" class="flex min-h-0 flex-1 flex-col">
    <header class="shrink-0 border-b border-border px-3 py-2">
      <div class="flex items-center gap-1.5">
        <button
          type="button"
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
          :disabled="parent === null"
          :title="t('files.up')"
          :aria-label="t('files.up')"
          @click="parent && go(parent)"
        >
          <ArrowUp :size="13" />
        </button>

        <nav class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto" :aria-label="t('files.path')">
          <template v-for="(crumb, index) in crumbs" :key="crumb.path">
            <ChevronRight v-if="index > 0" :size="11" class="shrink-0 text-muted-foreground" />
            <button
              type="button"
              :class="
                cn(
                  'shrink-0 rounded-sm px-1 py-0.5 font-mono text-[11px] transition-colors hover:bg-accent',
                  index === crumbs.length - 1 ? 'text-foreground' : 'text-muted-foreground',
                )
              "
              @click="go(crumb.path)"
            >
              {{ crumb.name }}
            </button>
          </template>
        </nav>

        <LoaderCircle v-if="browser.loading" :size="12" class="shrink-0 animate-spin text-muted-foreground" />

        <button
          type="button"
          class="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
          @click="startNewFolder()"
        >
          <FolderPlus :size="12" />
          {{ t("files.newFolder") }}
        </button>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
          @click="upload()"
        >
          <Upload :size="12" />
          {{ t("files.upload") }}
        </button>
      </div>

      <form class="mt-1.5 flex gap-1.5" @submit.prevent="submitTypedPath()">
        <input
          v-model="typedPath"
          type="text"
          spellcheck="false"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] focus:outline-none"
          :aria-label="t('files.path')"
          :placeholder="t('files.path')"
        />
        <button
          type="submit"
          class="shrink-0 rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent"
        >
          {{ t("files.go") }}
        </button>
      </form>

      <form
        v-if="typingNewFolder"
        class="mt-1.5 flex items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1.5"
        @submit.prevent="createFolder()"
      >
        <FolderPlus :size="12" class="shrink-0 text-muted-foreground" />
        <input
          v-model="newFolderName"
          type="text"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-[11px] focus:outline-none"
          :aria-label="t('files.newFolderName')"
          :placeholder="t('files.newFolderName')"
        />
        <button
          type="submit"
          class="shrink-0 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="busy || newFolderName.trim() === ''"
        >
          {{ t("files.create") }}
        </button>
        <button
          type="button"
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.cancel')"
          :aria-label="t('files.cancel')"
          @click="cancelPrompt()"
        >
          <X :size="11" />
        </button>
      </form>

      <form
        v-if="renaming !== null"
        class="mt-1.5 flex items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1.5"
        @submit.prevent="submitRename()"
      >
        <Pencil :size="12" class="shrink-0 text-muted-foreground" />
        <input
          v-model="renameText"
          type="text"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-[11px] focus:outline-none"
          :aria-label="t('files.rename.name')"
          :placeholder="t('files.rename.name')"
        />
        <button
          type="submit"
          class="shrink-0 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="busy || renameText.trim() === ''"
        >
          {{ t("files.rename.submit") }}
        </button>
        <button
          type="button"
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('files.cancel')"
          :aria-label="t('files.cancel')"
          @click="cancelPrompt()"
        >
          <X :size="11" />
        </button>
      </form>

      <div v-if="confirming" class="mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
        <p class="text-[11px]">
          {{
            confirming.kind === "directory"
              ? t("files.delete.confirmDirectory", { name: confirming.name })
              : t("files.delete.confirmFile", { name: confirming.name })
          }}
        </p>
        <p class="mt-1 text-[10px] leading-relaxed text-muted-foreground">{{ t("files.delete.note") }}</p>
        <div class="mt-2 flex gap-1">
          <button
            type="button"
            class="rounded-md bg-destructive px-2 py-1 text-[11px] text-destructive-foreground transition-colors hover:bg-destructive/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="busy"
            @click="remove(confirming)"
          >
            {{ t("files.delete.submit") }}
          </button>
          <button
            type="button"
            class="rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
            @click="cancelPrompt()"
          >
            {{ t("files.cancel") }}
          </button>
        </div>
      </div>

      <p v-if="problem" class="mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px]">
        <span>{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>
    </header>

    <div class="flex min-h-0 flex-1">
      <DirectoryTree v-if="twoPanes" :handle="props.handle" :current-path="browser.path" @navigate="goTo" />

      <div class="flex min-h-0 min-w-0 flex-1 flex-col">
        <div class="flex min-h-0 flex-1 flex-col overflow-auto">
          <div
            class="sticky top-0 flex items-center gap-2 border-b border-border bg-card/30 px-3 py-1.5 text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            <span class="min-w-0 flex-1">{{ t("files.name") }}</span>
            <span class="w-20 shrink-0 text-right">{{ t("files.size") }}</span>
            <span class="w-40 shrink-0 text-right">{{ t("files.modified") }}</span>
            <span class="w-28 shrink-0"></span>
          </div>

          <p
            v-if="!browser.loading && browser.entries.length === 0"
            class="px-3 py-6 text-center text-xs text-muted-foreground"
          >
            {{ t("files.empty") }}
          </p>

          <ul class="p-1.5">
            <li v-for="entry in browser.entries" :key="entry.path">
              <div
                :class="
                  cn(
                    'flex items-center gap-2 rounded-md px-1.5 py-1',
                    selected === entry.path ? 'bg-accent' : 'hover:bg-accent/50',
                  )
                "
              >
                <button
                  type="button"
                  class="flex min-w-0 flex-1 items-center gap-2 py-0.5 text-left"
                  :title="entry.name"
                  @click="selected = entry.path"
                  @dblclick="enter(entry)"
                >
                  <component :is="entryIcon(entry)" :size="13" class="shrink-0 text-muted-foreground" />
                  <span class="truncate text-xs">{{ entry.name }}</span>
                </button>

                <span class="w-20 shrink-0 text-right font-mono text-[10px] text-muted-foreground">{{
                  sizeOf(entry)
                }}</span>
                <span class="w-40 shrink-0 text-right text-[10px] text-muted-foreground">{{ modifiedOf(entry) }}</span>

                <span class="flex w-28 shrink-0 items-center justify-end gap-0.5">
                  <button
                    v-if="entry.kind === 'directory'"
                    type="button"
                    class="grid size-6 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                    :title="t('files.action.open')"
                    :aria-label="t('files.action.open')"
                    @click="enter(entry)"
                  >
                    <FolderOpen :size="12" />
                  </button>
                  <template v-else>
                    <button
                      v-if="isReadable(entry)"
                      type="button"
                      class="grid size-6 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                      :title="t('files.action.download')"
                      :aria-label="t('files.action.download')"
                      @click="download(entry)"
                    >
                      <Download :size="12" />
                    </button>
                    <button
                      v-if="isReadable(entry)"
                      type="button"
                      class="grid size-6 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                      :title="t('files.action.edit')"
                      :aria-label="t('files.action.edit')"
                      @click="editing = entry"
                    >
                      <FileText :size="12" />
                    </button>
                  </template>
                  <button
                    type="button"
                    class="grid size-6 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                    :title="t('files.action.rename')"
                    :aria-label="t('files.action.rename')"
                    @click="startRename(entry)"
                  >
                    <Pencil :size="12" />
                  </button>
                  <button
                    type="button"
                    class="grid size-6 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
                    :title="t('files.action.remove')"
                    :aria-label="t('files.action.remove')"
                    @click="startRemove(entry)"
                  >
                    <Trash2 :size="12" />
                  </button>
                </span>
              </div>
            </li>
          </ul>
        </div>

        <TransferList />
      </div>
    </div>

    <FileEditor
      :open="editing !== null"
      :handle="props.handle"
      :path="editing?.path ?? ''"
      @update:open="closeEditor"
    />
  </div>
</template>
