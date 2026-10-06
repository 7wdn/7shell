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
  File,
  FilePlus,
  FileSymlink,
  Folder,
  ClipboardPaste,
  FolderPlus,
  HardDrive,
  LoaderCircle,
  Pencil,
  Upload,
  X,
} from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import DirectoryTree from "@/components/files/DirectoryTree.vue";
import FileEditor from "@/components/files/FileEditor.vue";
import TransferList from "@/components/files/TransferList.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { formatMode, formatModified, formatSize, joinPath, parentPath } from "@/lib/files/format";
import { type Band, type Point, type RowBand, bandBetween, isDrag, rowsInBand, scrollStep } from "@/lib/files/marquee";
import {
  NOTHING_SELECTED,
  type Selection,
  clear,
  jumpLead,
  moveLead,
  retain,
  selectAll,
  selectOne,
  selectRange,
  selectedEntries,
  toggle,
} from "@/lib/files/selection";
import { fileAreaMenu, fileEntryMenu } from "@/lib/menu/files";
import { MENU_TITLES } from "@/lib/menu/titles";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app";
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

/**
 * The paste route, as a button.
 *
 * It uploads into the directory on screen. The Ctrl+V shortcut that goes with it is registered by
 * the shell rather than here, because "which directory is on screen" is a question a browser
 * cannot answer about itself: every open browser is mounted, including the ones behind other tabs,
 * and a shortcut registered by each of them would upload one clipboard into all of them at once.
 */
function pasteFiles(): void {
  const open = files.browser(props.handle);
  if (open) {
    void files.uploadFromClipboard(props.handle, open.path, preferences.packedTransfer);
  }
}

/** Goes where the tree asked, through the same path the listing uses. */
function goTo(path: string): void {
  void files.navigate(props.handle, path);
}

const { t } = useI18n();
const files = useFilesStore();
const { openMenu } = useContextMenu();
const app = useAppStore();

const browser = computed(() => files.browser(props.handle));
const problem = computed(() => (browser.value?.failure ? describeShellError(translate, browser.value.failure) : null));
const parent = computed(() => (browser.value ? parentPath(browser.value.path) : null));
const crumbs = computed(() => (browser.value ? crumbsOf(browser.value.path) : []));

/**
 * What is selected, and where the keyboard is.
 *
 * Held here rather than in the store because it is a view of this panel: the panes stay mounted, so
 * switching tabs and coming back finds the same rows selected, and nothing outside the listing has any
 * business knowing which they are.
 */
const selection = ref<Selection>(NOTHING_SELECTED);
/** The rectangle being dragged over the listing, in the listing's own coordinates. */
const marquee = ref<Band | null>(null);
const typedPath = ref("");
/** Which kind of thing is being named, and nothing while no prompt is open. */
const creating = ref<"folder" | "file" | null>(null);
const newName = ref("");
/** The path being renamed, so a server refusal can leave the typed name in place. */
const renaming = ref<string | null>(null);
const renameText = ref("");
/** The entries whose deletion is being confirmed. Never a bare click, and never only one at a time. */
const removal = ref<SftpEntry[] | null>(null);
const editing = ref<SftpEntry | null>(null);
const busy = ref(false);
/** The listing itself, which is what takes focus and carries the keyboard. */
const listing = ref<HTMLDivElement | null>(null);
/** A drag in progress over the empty space: where it started and what the rows measured. */
let drag: { from: Point; rows: RowBand[]; moved: boolean } | null = null;

/** The paths the listing is showing, in the order it draws them: what the selection is made of. */
const rows = computed(() => (browser.value?.entries ?? []).map((entry) => entry.path));
/** The selected entries, in listing order — what every action is about. */
const chosen = computed(() => selectedEntries(selection.value, browser.value?.entries ?? []));

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

/**
 * What kind of thing this is, in words.
 *
 * The icon says it as well, and the icon is what is read at a glance — but a listing that is being
 * read column by column needs the answer in the column, not in a shape the eye has to know.
 */
function typeOf(entry: SftpEntry): string {
  switch (entry.kind) {
    case "directory":
      return t("files.kind.directory");
    case "file":
      return t("files.kind.file");
    case "symlink":
      return t("files.kind.symlink");
    case "other":
      return t("files.kind.other");
  }
}

function modeOf(entry: SftpEntry): string {
  return formatMode(entry.mode);
}

/**
 * Who owns the entry, and the group it belongs to, in the one column they share.
 *
 * Whichever half the server did not send is left as a dash of its own rather than dropped: a row
 * that says `labuser/—` is saying the group is unknown, and one that says `labuser` is saying
 * something else.
 */
function ownershipOf(entry: SftpEntry): string {
  const owner = entry.owner ?? "—";
  const group = entry.group ?? "—";
  return `${owner}/${group}`;
}

/** Files and links can be downloaded or edited; a socket or a device cannot. */
function isReadable(entry: SftpEntry): boolean {
  return entry.kind === "file" || entry.kind === "symlink";
}

/**
 * A row's id, which is what `aria-activedescendant` names.
 *
 * By position rather than by path: a remote path holds spaces and quotes, and neither belongs in an
 * element id.
 */
function rowId(path: string): string {
  return `file-row-${rows.value.indexOf(path)}`;
}

watch(
  () => browser.value?.path,
  (path) => {
    typedPath.value = path ?? "";
  },
  { immediate: true },
);

// The listing reads itself again after every change, and a selection that outlived its rows would let
// "delete these three" name something that is no longer there.
watch(rows, (current) => {
  selection.value = retain(selection.value, current);
});

function enter(entry: SftpEntry): void {
  selection.value = selectOne(entry.path);
  open(entry);
}

/**
 * Opening one entry: a directory is walked into, a file is read in the editor.
 *
 * The listing has no way to show a file, so "opening" one means the editor — which is also what a
 * person means by double-clicking a text file in a file manager.
 */
function open(entry: SftpEntry): void {
  if (entry.kind === "directory") {
    go(entry.path);
    return;
  }
  if (isReadable(entry)) {
    editing.value = entry;
  }
}

function go(path: string): void {
  selection.value = clear();
  void files.navigate(props.handle, path);
}

/**
 * A click on a row, in whichever of the three shapes the modifiers make.
 *
 * This is where the platform's conventions live: a plain click takes that row alone, a shift-click
 * takes everything between the last plain click and here, and a ctrl-click adds or removes one row.
 * The platform matters for the last of those — on a Mac the modifier is Command, because **Ctrl+click
 * is the secondary click there** and stealing it would take the context menu away.
 */
function clickRow(event: MouseEvent, entry: SftpEntry): void {
  if (event.shiftKey) {
    selection.value = selectRange(selection.value, rows.value, entry.path);
  } else if (modifies(event)) {
    selection.value = toggle(selection.value, entry.path);
  } else {
    selection.value = selectOne(entry.path);
  }
  // The keyboard is now in the listing, so an arrow key straight afterwards walks these rows.
  listing.value?.focus();
}

/** Cmd on a Mac, Ctrl everywhere else. */
function modifies(event: MouseEvent | KeyboardEvent): boolean {
  // The platform arrives as "macos aarch64" and the like, so this asks what it starts with rather
  // than whether it is equal to anything.
  return app.platform.startsWith("macos") ? event.metaKey : event.ctrlKey;
}

/**
 * The listing's keyboard.
 *
 * Everything here is what a file manager does: arrows walk (shift extends), the ends jump, ctrl-A
 * takes all, Delete asks about the selection, F2 renames the one row, Enter opens it, Escape lets go.
 * The one thing that does not do what a browser does is Ctrl-A: the listing is a list, not a document,
 * and "select all the files" is what somebody means by it.
 */
function onListingKeydown(event: KeyboardEvent): void {
  const paths = rows.value;

  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      selection.value = moveLead(selection.value, paths, 1, event.shiftKey);
      return;
    case "ArrowUp":
      event.preventDefault();
      selection.value = moveLead(selection.value, paths, -1, event.shiftKey);
      return;
    case "Home":
      event.preventDefault();
      selection.value = jumpLead(selection.value, paths, "first", event.shiftKey);
      return;
    case "End":
      event.preventDefault();
      selection.value = jumpLead(selection.value, paths, "last", event.shiftKey);
      return;
    case "a":
    case "A":
      if (modifies(event)) {
        event.preventDefault();
        selection.value = selectAll(paths);
      }
      return;
    case "Escape":
      event.preventDefault();
      selection.value = clear();
      return;
    case "Delete":
      event.preventDefault();
      if (chosen.value.length > 0) {
        startRemove(chosen.value);
      }
      return;
    case "F2": {
      const only = chosen.value.length === 1 ? chosen.value[0] : undefined;
      if (only !== undefined) {
        event.preventDefault();
        startRename(only);
      }
      return;
    }
    case "Enter": {
      const only = chosen.value.length === 1 ? chosen.value[0] : undefined;
      if (only !== undefined) {
        event.preventDefault();
        open(only);
      }
      return;
    }
    default:
      return;
  }
}

/** Where a pointer is, in the listing's own coordinates: the ones its contents are measured in. */
function contentPoint(event: PointerEvent): Point {
  const element = listing.value;
  if (!element) {
    return { x: event.clientX, y: event.clientY };
  }
  const box = element.getBoundingClientRect();
  return { x: event.clientX - box.left, y: event.clientY - box.top + element.scrollTop };
}

/**
 * The rows as they are laid out right now.
 *
 * Measured once, when a drag begins: the rows do not move while a rectangle is drawn over them, and
 * measuring a directory's worth of rows on every pointer move is how a drag turns into a slideshow.
 */
function measureRows(): RowBand[] {
  const element = listing.value;
  if (!element) {
    return [];
  }
  return [...element.querySelectorAll<HTMLElement>("li[data-path]")].map((row) => ({
    path: row.dataset.path ?? "",
    top: row.offsetTop,
    bottom: row.offsetTop + row.offsetHeight,
  }));
}

/**
 * A press on the empty space under the rows.
 *
 * Two gestures start here and which one it is is decided by whether the pointer moves: a click means
 * "none of these" and clears the selection, a drag draws a rectangle. Deciding it at the *end* — on
 * pointer up — is what keeps a click from being swallowed by a rectangle that was never drawn.
 */
function onListingPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || marquee.value !== null) {
    return;
  }
  const target = event.target as HTMLElement | null;
  // A row, a button or the column headings: all of them have their own meaning for a press, and none
  // of them is "empty space".
  if (target?.closest("li, button, input, [data-listing-header]")) {
    return;
  }

  drag = { from: contentPoint(event), rows: measureRows(), moved: false };
  window.addEventListener("pointermove", onListingPointerMove);
  window.addEventListener("pointerup", onListingPointerUp);
}

function onListingPointerMove(event: PointerEvent): void {
  const element = listing.value;
  if (!drag || !element) {
    return;
  }

  const here = { x: event.clientX, y: event.clientY };
  if (!drag.moved && !isDrag(drag.from, here)) {
    return;
  }
  drag.moved = true;

  const band = bandBetween(drag.from, contentPoint(event));
  marquee.value = band;
  const covered = rowsInBand(drag.rows, band);
  selection.value = { paths: covered, anchor: covered[0] ?? null, lead: covered[covered.length - 1] ?? null };

  // Held past an edge, the listing keeps moving — otherwise a rectangle could never reach a row that
  // is not on screen.
  const box = element.getBoundingClientRect();
  const step = scrollStep(here.y, { top: box.top, height: box.height });
  if (step !== 0) {
    element.scrollTop += step;
  }
}

function onListingPointerUp(): void {
  window.removeEventListener("pointermove", onListingPointerMove);
  window.removeEventListener("pointerup", onListingPointerUp);
  const dragged = drag?.moved ?? false;
  drag = null;
  marquee.value = null;

  if (!dragged) {
    // A click that went nowhere: the person meant "none of these".
    selection.value = clear();
  }
}

function startNew(kind: "folder" | "file"): void {
  cancelPrompt();
  newName.value = "";
  creating.value = kind;
}

function startRename(entry: SftpEntry): void {
  cancelPrompt();
  renaming.value = entry.path;
  renameText.value = entry.name;
}

/** Asking to delete: the entries are collected once, and the question is asked over everything else. */
function startRemove(entries: readonly SftpEntry[]): void {
  cancelPrompt();
  removal.value = [...entries];
}

function submitTypedPath(): void {
  const path = typedPath.value.trim();
  if (path !== "") {
    go(path);
  }
}

function cancelPrompt(): void {
  creating.value = null;
  renaming.value = null;
}

/**
 * Creates what was named, and — for a file — opens it.
 *
 * A file that is created and then left in the list is a file nobody asked for: the list has no way to
 * open one, so the editor is the only place a new empty file is of any use. A folder is its own
 * answer, and opening anything on its behalf would be presumptuous.
 */
async function createEntry(): Promise<void> {
  const current = browser.value;
  const kind = creating.value;
  const name = newName.value.trim();
  if (!current || kind === null || name === "") {
    return;
  }

  const path = joinPath(current.path, name);
  busy.value = true;
  const worked =
    kind === "folder" ? await files.createFolder(props.handle, path) : await files.createFile(props.handle, path);
  busy.value = false;
  if (!worked) {
    return;
  }

  creating.value = null;
  newName.value = "";
  if (kind === "file") {
    editing.value = { name, path, kind: "file", size: 0 };
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

async function remove(entries: readonly SftpEntry[]): Promise<void> {
  busy.value = true;
  const worked = await files.removeMany(props.handle, entries);
  busy.value = false;
  if (worked) {
    removal.value = null;
  }
}

/**
 * What the question says: the action as a name, and what it takes with it.
 *
 * A folder is not a file and the question does not pretend otherwise — "everything under it" is in
 * the sentence, because that is the part somebody has to have read before answering. The button
 * repeats the name rather than saying "OK".
 */
const removalAction = computed(() => {
  const entries = removal.value ?? [];
  if (entries.length > 1) {
    return t("files.delete.items", { count: entries.length });
  }
  return entries[0]?.kind === "directory" ? t("files.delete.folder") : t("files.delete.file");
});

const removalMessage = computed(() => {
  const entries = removal.value ?? [];
  if (entries.length > 1) {
    return t("files.delete.itemsMessage", { count: entries.length });
  }
  const one = entries[0];
  return one?.kind === "directory"
    ? t("files.delete.folderMessage", { name: one.name })
    : t("files.delete.fileMessage", { name: one?.name ?? "" });
});

/** The paths themselves, for a question the sentence cannot name one by one. */
const removalDetails = computed(() =>
  (removal.value?.length ?? 0) > 1 ? (removal.value ?? []).map((entry) => entry.path) : [],
);

/**
 * Taking what is selected, whichever way the switch says it travels.
 *
 * One door to the store and one command behind it: the backend asks which folder whenever it needs
 * one, and whether several rows become a folder's worth of transfers or a single archive is decided
 * by the switch and the count rather than by two different entries in this menu.
 */
function download(entries: readonly SftpEntry[], packed: boolean): void {
  if (entries.length === 0) {
    return;
  }
  const one = entries.length === 1 ? entries[0] : undefined;
  // A folder is being asked for unless there is exactly one file to name, and a folder picker has no
  // use for a suggested file name.
  const naming = !packed && entries.length === 1 ? (one?.name ?? "") : "";
  void files.download(
    props.handle,
    entries.map((entry) => entry.path),
    naming === "" ? t("files.downloadPackedTitle") : t("files.downloadTitle"),
    naming,
    packed,
  );
}

function upload(): void {
  const current = browser.value;
  if (current) {
    // The same button, two jobs: with packing on it is several files that are chosen, and they are
    // packed here before anything crosses the wire.
    const packed = preferences.packedTransfer;
    void files.upload(
      props.handle,
      current.path,
      packed ? t("files.uploadPackedTitle") : t("files.uploadTitle"),
      packed,
    );
  }
}

function closeEditor(open: boolean): void {
  if (!open) {
    editing.value = null;
  }
}

/**
 * The menu for the rows somebody is pointing at, built as it is opened.
 *
 * A right-click on a row that is *not* selected makes that row the selection first — which is how a
 * menu over a row that was not chosen still answers about the thing under the pointer. A right-click
 * on a row that *is* selected keeps the whole selection, because that is what a menu over several
 * chosen rows means everywhere else.
 */
function openEntryMenu(event: MouseEvent, entry: SftpEntry): void {
  if (!selection.value.paths.includes(entry.path)) {
    selection.value = selectOne(entry.path);
  }
  const entries = chosen.value;

  openMenu(
    event,
    entries.length > 1
      ? { key: MENU_TITLES.files, params: { count: String(entries.length) } }
      : { key: MENU_TITLES.file, params: { name: entry.name } },
    fileEntryMenu(
      entries,
      {
        open: enter,
        download,
        edit: (target) => {
          editing.value = target;
        },
        rename: startRename,
        remove: startRemove,
      },
      { packed: preferences.packedTransfer },
    ),
  );
}

/**
 * The menu for the directory on screen, opened from the listing's empty space.
 *
 * The whole scroll area carries it, the sticky column headings included: those headings are part of
 * the same listing, and a strip a few pixels tall that swallowed a right-click would be worse than one
 * that offers the directory's actions.
 */
function openAreaMenu(event: MouseEvent): void {
  const current = browser.value;
  if (!current) {
    return;
  }
  openMenu(
    event,
    { key: MENU_TITLES.directory },
    fileAreaMenu(
      { path: current.path, packed: preferences.packedTransfer },
      {
        newFolder: () => startNew("folder"),
        newFile: () => startNew("file"),
        upload,
        paste: pasteFiles,
        refresh: () => void files.navigate(props.handle, current.path),
        togglePacked: () => void preferences.setPackedTransfer(!preferences.packedTransfer),
      },
    ),
  );
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
          @click="startNew('folder')"
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

        <button
          type="button"
          class="flex shrink-0 items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
          :title="t('files.paste.title')"
          @click="pasteFiles()"
        >
          <ClipboardPaste :size="11" />
          {{ t("files.paste.label") }}
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
        v-if="creating !== null"
        class="mt-1.5 flex items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1.5"
        @submit.prevent="createEntry()"
      >
        <FolderPlus v-if="creating === 'folder'" :size="12" class="shrink-0 text-muted-foreground" />
        <FilePlus v-else :size="12" class="shrink-0 text-muted-foreground" />
        <input
          v-model="newName"
          type="text"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-[11px] focus:outline-none"
          :aria-label="creating === 'folder' ? t('files.newFolderName') : t('files.newFileName')"
          :placeholder="creating === 'folder' ? t('files.newFolderName') : t('files.newFileName')"
        />
        <button
          type="submit"
          class="shrink-0 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="busy || newName.trim() === ''"
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

      <p v-if="problem" class="mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px]">
        <span>{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>
    </header>

    <div class="flex min-h-0 flex-1">
      <DirectoryTree v-if="twoPanes" :handle="props.handle" :current-path="browser.path" @navigate="goTo" />

      <div class="flex min-h-0 min-w-0 flex-1 flex-col">
        <div
          ref="listing"
          role="listbox"
          aria-multiselectable="true"
          tabindex="0"
          :aria-label="t('files.listing')"
          :aria-activedescendant="selection.lead === null ? undefined : rowId(selection.lead)"
          :class="
            cn(
              'relative flex min-h-0 flex-1 flex-col overflow-auto outline-none',
              // A listing is not a text document, and the browser treats anything selectable as one:
              // a shift-click extends the *text* selection from wherever the last click landed, and
              // the pointer turns into an I-beam while it is over the names. Both are wrong here — the
              // rows are what is being chosen — so the container refuses to be selectable at all.
              'cursor-default select-none',
            )
          "
          @keydown="onListingKeydown"
          @pointerdown="onListingPointerDown"
          @contextmenu="openAreaMenu"
        >
          <div
            data-listing-header
            class="sticky top-0 flex items-center gap-2 border-b border-border bg-card/30 px-3 py-1.5 text-[10px] font-medium tracking-wide text-muted-foreground uppercase"
          >
            <span class="min-w-0 flex-1">{{ t("files.name") }}</span>
            <span class="w-20 shrink-0 text-right">{{ t("files.size") }}</span>
            <span class="w-16 shrink-0">{{ t("files.type") }}</span>
            <span class="w-40 shrink-0 text-right">{{ t("files.modified") }}</span>
            <span class="w-20 shrink-0">{{ t("files.permissions") }}</span>
            <span class="w-28 shrink-0">{{ t("files.owner") }}</span>
          </div>

          <p
            v-if="!browser.loading && browser.entries.length === 0"
            class="px-3 py-6 text-center text-xs text-muted-foreground"
          >
            {{ t("files.empty") }}
          </p>

          <ul class="p-1.5">
            <li v-for="entry in browser.entries" :key="entry.path" :data-path="entry.path">
              <div
                :id="rowId(entry.path)"
                role="option"
                :aria-selected="selection.paths.includes(entry.path)"
                :class="
                  cn(
                    'flex items-center gap-2 rounded-md px-1.5 py-1',
                    selection.paths.includes(entry.path) ? 'bg-accent' : 'hover:bg-accent/50',
                    // The row the keyboard is on, which is not always an obvious one once shift has
                    // been used: a ring says where the next arrow key starts.
                    selection.lead === entry.path && 'ring-1 ring-primary/50 ring-inset',
                  )
                "
                @click="clickRow($event, entry)"
                @dblclick="enter(entry)"
                @contextmenu="openEntryMenu($event, entry)"
              >
                <span class="flex min-w-0 flex-1 items-center gap-2 py-0.5 text-left" :title="entry.name">
                  <component :is="entryIcon(entry)" :size="13" class="shrink-0 text-muted-foreground" />
                  <span class="truncate text-xs">{{ entry.name }}</span>
                </span>

                <span class="w-20 shrink-0 text-right font-mono text-[10px] text-muted-foreground">{{
                  sizeOf(entry)
                }}</span>
                <span class="w-16 shrink-0 truncate text-[10px] text-muted-foreground">{{ typeOf(entry) }}</span>
                <span class="w-40 shrink-0 text-right text-[10px] text-muted-foreground">{{ modifiedOf(entry) }}</span>
                <span class="w-20 shrink-0 font-mono text-[10px] text-muted-foreground">{{ modeOf(entry) }}</span>
                <span
                  class="w-28 shrink-0 truncate font-mono text-[10px] text-muted-foreground"
                  :title="ownershipOf(entry)"
                  >{{ ownershipOf(entry) }}</span
                >
              </div>
            </li>
          </ul>

          <!--
            The rectangle being dragged, drawn inside the scroller so it travels with the rows it is
            covering. It never takes a pointer: the drag belongs to the listing underneath it.
          -->
          <div
            v-if="marquee"
            class="pointer-events-none absolute inset-x-0 border border-primary/50 bg-primary/10"
            :style="{ top: `${marquee.top}px`, height: `${marquee.bottom - marquee.top}px` }"
          ></div>
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

    <!--
      The question, over the panel rather than inside its header: what is being asked about is the
      rows underneath, and the header is where the panel's own writing happens. While the removal
      runs the dialog stays up and says so — a recursive delete is not instant, and a row that
      simply vanished would leave the question of whether it worked.
    -->
    <ConfirmDialog
      v-if="removal"
      :open="true"
      :title="removalAction"
      :message="removalMessage"
      :details="removalDetails"
      :confirm-label="removalAction"
      danger
      :loading="busy"
      @confirm="remove(removal)"
      @update:open="removal = null"
    />
  </div>
</template>
