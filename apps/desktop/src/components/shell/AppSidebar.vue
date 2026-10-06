<script setup lang="ts">
/**
 * The session list, the forwards that are up, and the protocols this build speaks.
 *
 * The list is what is saved, grouped by the groups that are saved. Nothing here invents a row: a
 * session whose group no longer exists still appears, at the root, rather than disappearing because
 * its heading did.
 *
 * Groups are managed where they are drawn. Making one, renaming one and deleting one all happen in
 * the list rather than in a dialog, and deleting asks in the heading the way deleting a session asks
 * in its row. A group is only ever a heading: nothing inside it is deleted with it, which is the
 * sentence the confirmation says out loud.
 *
 * A session that is in no group is drawn at the root with no heading over it — the rows say what
 * they are. What the heading was also for is drawn only while a session is being dragged: a row at
 * the end of the list that means "out of every group", there when somebody needs it and not part of
 * the list the rest of the time. Dragging is off while the list is filtered, because a drop lands
 * in the list as it is *drawn*, and when that order is not the saved order the drop would be a
 * guess.
 *
 * Opening a terminal hangs off the row itself, with the file browser beside it; whether a terminal
 * comes up with its file panel is the settings' answer, not this row's. Editing and deleting are
 * separate buttons, and deleting always asks first, in the row, before anything is removed.
 *
 * The tunnels section draws what the terminals store was told by the backend, and only then: an
 * empty section is not shown at all. A forward that failed to come up is shown next to the ones
 * that are, because "the tunnel is not there" is the answer somebody staring at a port needs.
 */
import {
  ArrowRight,
  Check,
  ChevronRight,
  Download,
  EllipsisVertical,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  ListFilter,
  LoaderCircle,
  Network,
  Pencil,
  Plus,
  Search,
  Server,
  SlidersHorizontal,
  Terminal,
  Trash2,
  TriangleAlert,
  Upload,
  X,
} from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ExportSessionsDialog from "@/components/sessions/ExportSessionsDialog.vue";
import GroupNameForm from "@/components/sessions/GroupNameForm.vue";
import SessionDialog from "@/components/sessions/SessionDialog.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { useToast } from "@/composables/useToast";
import { useTreeDrag } from "@/composables/useTreeDrag";
import { describeShellError } from "@/i18n/errors";
import { groupMenu, sessionMenu, tunnelMenu } from "@/lib/menu/sessions";
import { MENU_TITLES } from "@/lib/menu/titles";
import { groupDropFor, sessionDropFor } from "@/lib/sessions/drag";
import { hasSibling } from "@/lib/sessions/groups";
import { rowTarget } from "@/lib/sessions/tree";
import { useAppStore } from "@/stores/app";
import { useSessionsStore } from "@/stores/sessions";
import { tunnelFailureKey, useTerminalsStore } from "@/stores/terminals";
import type { OpenTunnel, Session, SessionGroup, SessionImportSummary } from "@/types/session";
import { AUTH_METHODS, PROTOCOLS } from "@/types/generated/protocols";

const { t } = useI18n();
const app = useAppStore();
const library = useSessionsStore();
const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();
const { toast } = useToast();

const protocols = computed(() => PROTOCOLS);
/** The ways a saved session can authenticate, which is what the list can be narrowed by. */
const authMethods = computed(() => AUTH_METHODS);
const editorOpen = ref(false);
const editing = ref<Session | null>(null);

/**
 * The name an export is offered under.
 *
 * A fixed name rather than a translated one: it is a file name, and a file name that changes with
 * the interface language is one that cannot be found again in a folder listing.
 */
const SESSION_FILE_NAME = "7shell-sessions.json";

/** The row that exists only while a session is being dragged: where "out of every group" is. */
const DROP_OUT_ROW = "drop-out";

const exportOpen = ref(false);
const exportBusy = ref(false);

/** The filter panel, and whether the panel the button opens is up. */
const filterOpen = ref(false);
const filterBox = ref<HTMLElement | null>(null);
const filterButton = ref<HTMLElement | null>(null);

/**
 * The removal somebody has asked for and not yet answered.
 *
 * One value for both kinds rather than one each: a question is about a single thing at a time, and
 * two of them on screen at once would be two dialogs stacked over each other. A trash button and a
 * menu entry both land here; nothing is ever removed by the click that asks.
 */
type PendingRemoval =
  | { readonly kind: "session"; readonly session: Session }
  | { readonly kind: "group"; readonly group: SessionGroup };
const pendingRemoval = ref<PendingRemoval | null>(null);

/** The group prompts, of which at most one is open: a name being made, or one being changed. */
const creatingGroup = ref(false);
/** The group a new one is being made inside, or null for the root of the tree. */
const creatingInside = ref<string | null>(null);
const newGroupName = ref("");
/** The naming form that is open, whichever of the places it is drawn in. */
const openNameForm = ref<{ focus: () => void } | null>(null);
const renamingGroup = ref<string | null>(null);
const groupNameText = ref("");
/** The field drawn over a group's name while it is being renamed, wherever in the list that is. */
const renameField = ref<HTMLInputElement | null>(null);
/** One group write at a time, so pressing Create twice does not make two groups. */
const groupBusy = ref(false);

/** The scroller the rows live in: what a drag is hit-tested against, and what it scrolls. */
const listBox = ref<HTMLElement | null>(null);
const rowByKey = computed(() => new Map(library.rows.map((row) => [row.key, row])));
const {
  active: dragActive,
  grabbed: dragGrabbed,
  ghost: dragGhost,
  mark: dropMark,
  press: pressRow,
} = useTreeDrag({
  container: listBox,
  targetOf: (element) => {
    const key = element.dataset.row ?? "";
    // The row that is there only while a drag is: it stands for the root of the list, which is
    // where a session goes when it leaves every group. It is not in the tree, so it is answered for
    // here rather than looked up.
    if (key === DROP_OUT_ROW) {
      return { kind: "session", parentId: null };
    }
    const row = rowByKey.value.get(key);
    return row ? rowTarget(row) : null;
  },
  aim: (grabbed, target, box, pointerY) =>
    grabbed.kind === "group" ? groupDropFor(target, box, pointerY) : sessionDropFor(target),
  accepts: (grabbed, aimed) =>
    grabbed.kind === "group"
      ? library.canPlaceGroup(grabbed.id, aimed.drop, aimed.zone)
      : library.canPlaceSession(grabbed.id, aimed.drop),
  onDrop: (grabbed, aimed) => {
    if (grabbed.kind === "group") {
      void library.placeGroup(grabbed.id, aimed.drop, aimed.zone);
    } else {
      void library.placeSession(grabbed.id, aimed.drop);
    }
  },
});

/**
 * Whether the row for "out of every group" belongs on screen: a session is in the air, and there is
 * a group for it to be outside of — with no groups at all, everything is already at the root.
 */
const dropOutRow = computed(
  () => dragActive.value && dragGrabbed.value?.kind === "session" && library.groups.length > 0,
);

/**
 * Starts a drag, unless the list is narrowed.
 *
 * A drop lands in the list as it is drawn, so while a filter is on the row under the pointer is not
 * the row the store would move something next to. Rather than guessing, the drag is not offered —
 * the menus still move things, and clearing the search brings it back.
 */
function startDrag(event: MouseEvent, item: { kind: "group" | "session"; id: string }, name: string): void {
  if (library.filtering) {
    return;
  }
  pressRow(event, item, name);
}

const problem = computed(() => (library.failure ? describeShellError(translate, library.failure) : null));
const tunnelProblem = computed(() =>
  terminals.tunnelProblem ? describeShellError(translate, terminals.tunnelProblem) : null,
);
const hasTunnels = computed(
  () => terminals.tunnels.length > 0 || terminals.tunnelFailures.length > 0 || tunnelProblem.value !== null,
);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function endpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

function startNew(): void {
  editing.value = null;
  editorOpen.value = true;
}

function startEdit(session: Session): void {
  editing.value = session;
  editorOpen.value = true;
}

/**
 * Keeps the filter panel open while it is being used, and closes it when the pointer goes anywhere
 * else.
 *
 * A panel of checkboxes that closes on the first one would be a panel somebody has to open once per
 * method, so the click that chooses is not the click that closes; the click outside is.
 */
function closeFilterOnOutsideClick(event: MouseEvent): void {
  const target = event.target;
  if (!(target instanceof Node)) {
    return;
  }
  if (filterBox.value?.contains(target) || filterButton.value?.contains(target)) {
    return;
  }
  filterOpen.value = false;
}

watch(filterOpen, (open) => {
  if (open) {
    document.addEventListener("mousedown", closeFilterOnOutsideClick, true);
  } else {
    document.removeEventListener("mousedown", closeFilterOnOutsideClick, true);
  }
});

onBeforeUnmount(() => document.removeEventListener("mousedown", closeFilterOnOutsideClick, true));

/**
 * What an import did, said as a receipt.
 *
 * The counts are joined rather than fixed into one sentence because what came out of a file is not
 * known in advance: an empty import, a plain one, and one that carried a warning all have to read
 * as the same kind of sentence. Anything that could not be carried is named here or nowhere — the
 * session is in the list either way, and this is the only moment anybody can be told why it might
 * not connect the way it did where the file came from.
 */
function importReceipt(summary: SessionImportSummary): string {
  const parts: string[] = [];
  const counted = (key: string, count: number): string => t(key, { count }, count);
  if (summary.sessions > 0) {
    parts.push(counted("sessions.list.importSessions", summary.sessions));
  }
  if (summary.groups > 0) {
    parts.push(counted("sessions.list.importGroups", summary.groups));
  }
  if (summary.secrets > 0) {
    parts.push(counted("sessions.list.importSecrets", summary.secrets));
  }
  if (summary.missingKeys > 0) {
    parts.push(counted("sessions.list.importMissingKeys", summary.missingKeys));
  }
  if (summary.droppedHops > 0) {
    parts.push(counted("sessions.list.importDroppedHops", summary.droppedHops));
  }
  return parts.length === 0
    ? t("sessions.list.importNothing")
    : t("sessions.list.importDone", { parts: parts.join(", ") });
}

/** Reads a file the user picks and says what it added. A refusal stays in the list's own box. */
async function runImport(): Promise<void> {
  const summary = await library.importLibrary(t("sessions.list.importTitle"));
  if (!summary) {
    return;
  }
  toast(importReceipt(summary), summary.missingKeys + summary.droppedHops > 0 ? "result" : "confirmation");
}

/** Writes the library to a file the user picks. */
async function runExport(includeSecrets: boolean): Promise<void> {
  exportBusy.value = true;
  const summary = await library.exportLibrary(t("sessions.list.exportTitle"), SESSION_FILE_NAME, includeSecrets);
  exportBusy.value = false;
  // Closed either way: a dismissed file dialog and a failed write are both "nothing more to decide
  // here", and a failure is already drawn in the list's own box.
  exportOpen.value = false;
  if (summary) {
    toast(
      t("sessions.list.exportDone", { count: summary.sessions, file: summary.fileName }, summary.sessions),
      "receipt",
    );
  }
}

/** The library's own menu: the two things that are done to the whole list rather than to a row. */
function openLibraryMenu(event: MouseEvent): void {
  openMenu(event, { key: MENU_TITLES.library }, [
    {
      kind: "action",
      id: "import-sessions",
      labelKey: "sessions.list.import",
      icon: Download,
      run: () => void runImport(),
    },
    {
      kind: "action",
      id: "export-sessions",
      labelKey: "sessions.list.export",
      icon: Upload,
      disabled: library.sessions.length === 0,
      run: () => {
        exportOpen.value = true;
      },
    },
  ]);
}

/**
 * What the confirmation says, for whichever kind of thing was asked about.
 *
 * The title and the button carry the same words — "Delete group" — because the button that finishes
 * the sentence the title started is the one nobody presses by reflex. The message says what happens
 * to what is inside: a group is not a folder, and the sessions in it outlive it.
 */
const removal = computed(() => {
  const asked = pendingRemoval.value;
  if (!asked) {
    return null;
  }
  return asked.kind === "session"
    ? {
        title: t("sessions.list.remove"),
        message: t("sessions.list.removeMessage", { name: asked.session.name }),
      }
    : {
        title: t("sessions.list.groupRemove"),
        message: t("sessions.list.groupRemoveMessage", { name: asked.group.name }),
      };
});

/** The answer was yes: the question goes away first, then the removal runs. */
async function confirmRemoval(): Promise<void> {
  const asked = pendingRemoval.value;
  pendingRemoval.value = null;
  if (!asked) {
    return;
  }
  await (asked.kind === "session" ? library.remove(asked.session.id) : removeGroup(asked.group));
}

function cancelGroupPrompts(): void {
  creatingGroup.value = false;
  creatingInside.value = null;
  renamingGroup.value = null;
}

/**
 * Opens the naming form: under the list's own header for a group at the root, or under the heading
 * it will sit inside. The cursor goes into it, because a form that was just opened is a form
 * somebody asked to type in.
 */
async function startGroupCreate(parentId: string | null): Promise<void> {
  cancelGroupPrompts();
  newGroupName.value = "";
  creatingInside.value = parentId;
  creatingGroup.value = true;
  await nextTick();
  openNameForm.value?.focus();
}

/**
 * The open naming form hands itself over through here.
 *
 * A `ref` written on an element inside a list arrives as an array of everything the list rendered,
 * and there is one form here, drawn in whichever of two places applies — so the form is taken as it
 * comes and the one that is mounted is the one that gets the cursor.
 */
function takeNameForm(form: unknown): void {
  openNameForm.value = (form as { focus: () => void } | null) ?? null;
}

function startGroupRename(group: SessionGroup): void {
  cancelGroupPrompts();
  groupNameText.value = group.name;
  renamingGroup.value = group.id;
  void nextTick().then(() => {
    renameField.value?.focus();
    renameField.value?.select();
  });
}

/**
 * The rename field hands itself over through here, for the same reason the naming form does: a
 * `ref` inside a list arrives as an array, and there is only ever one field on screen.
 */
function takeRenameField(field: unknown): void {
  renameField.value = (field as HTMLInputElement | null) ?? null;
}

async function createGroup(): Promise<void> {
  const name = newGroupName.value.trim();
  if (name === "" || groupBusy.value) {
    return;
  }
  groupBusy.value = true;
  const worked = await library.addGroup(name, creatingInside.value);
  groupBusy.value = false;
  if (worked) {
    creatingGroup.value = false;
    creatingInside.value = null;
    newGroupName.value = "";
  }
}

/**
 * Writes the name that was typed over the old one.
 *
 * Three rules, all of them from how a rename in place is used: the editing closes **before** the
 * write, so the blur that follows Enter cannot run the same rename twice; an empty name, or the
 * name it already had, is a cancel rather than a write; and a refusal puts the field back with what
 * was typed still in it — the reason is drawn in the list's own problem box, and retyping a name
 * somebody just typed is not a thing to ask for.
 */
async function submitGroupRename(group: SessionGroup): Promise<void> {
  if (renamingGroup.value !== group.id) {
    return;
  }
  renamingGroup.value = null;
  const name = groupNameText.value.trim();
  if (name === "" || name === group.name || groupBusy.value) {
    return;
  }

  groupBusy.value = true;
  const worked = await library.saveGroup({ ...group, name });
  groupBusy.value = false;
  if (!worked) {
    renamingGroup.value = group.id;
    await nextTick();
    renameField.value?.focus();
  }
}

/** A group is written one at a time, and removing one is a group write like any other. */
async function removeGroup(group: SessionGroup): Promise<void> {
  groupBusy.value = true;
  await library.removeGroup(group.id);
  groupBusy.value = false;
}

/** How far a row is indented: one step per level of nesting. */
function indent(depth: number): Record<string, string> {
  return { paddingLeft: `${4 + depth * 12}px` };
}

/** How a row is drawn while it is the place a drag would land: a line beside, or a ring around. */
function markOf(key: string): string | null {
  return dropMark.value?.key === key ? dropMark.value.style : null;
}

/**
 * The menus this sidebar offers: one for a saved session, one for a group heading, and one for a
 * forward that is up.
 *
 * Deleting from the menu asks the same question the trash button asks rather than doing it — the
 * menu only decides *what* is being removed, and the one dialog decides whether it is.
 */
function openSessionMenu(event: MouseEvent, session: Session): void {
  openMenu(
    event,
    { key: MENU_TITLES.session },
    sessionMenu(session, library.groups, {
      openTerminal: (target) => void terminals.open(target),
      openFiles: (target) => void terminals.openFiles(target),
      edit: startEdit,
      remove: (target) => {
        pendingRemoval.value = { kind: "session", session: target };
      },
      move: (target, groupId) =>
        void library.placeSession(target.id, groupId === undefined ? { kind: "root" } : { kind: "group", id: groupId }),
    }),
  );
}

/** The ungrouped heading is not a thing anybody can rename or move, so it has no menu. */
function openHeadingMenu(event: MouseEvent, group: SessionGroup | null): void {
  if (!group) {
    return;
  }
  openMenu(
    event,
    { key: MENU_TITLES.group },
    groupMenu(
      group,
      {
        rename: startGroupRename,
        remove: (target) => {
          pendingRemoval.value = { kind: "group", group: target };
        },
        move: (target, direction) => void library.moveGroupBy(target.id, direction),
        addInside: (target) => void startGroupCreate(target.id),
      },
      {
        up: hasSibling(library.groups, group.id, -1),
        down: hasSibling(library.groups, group.id, 1),
      },
    ),
  );
}

function openTunnelMenu(event: MouseEvent, tunnel: OpenTunnel): void {
  openMenu(event, { key: MENU_TITLES.tunnel }, tunnelMenu(tunnel, { stop: (id) => void terminals.stopTunnel(id) }));
}

const statusLabel = computed(() => {
  switch (app.status) {
    case "loading":
      return t("common.status.loading");
    case "failed":
      return t("common.status.failed");
    default:
      return t("common.status.ready");
  }
});

const statusDot = computed(() => {
  if (app.status === "failed") {
    return "bg-destructive";
  }
  return app.status === "ready" ? "bg-success" : "bg-warning";
});
</script>

<template>
  <aside class="flex w-64 shrink-0 flex-col border-r border-border bg-card/30">
    <section class="flex min-h-0 flex-1 flex-col px-3 pt-3">
      <header class="mb-2 flex shrink-0 items-center justify-between">
        <h2 class="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("shell.sidebar.sessions") }}
        </h2>
        <span class="flex items-center gap-0.5">
          <button
            type="button"
            class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('sessions.list.addGroup')"
            :aria-label="t('sessions.list.addGroup')"
            @click="startGroupCreate(null)"
          >
            <FolderPlus :size="14" />
          </button>
          <button
            type="button"
            class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('sessions.list.add')"
            :aria-label="t('sessions.list.add')"
            @click="startNew()"
          >
            <Plus :size="14" />
          </button>
          <button
            type="button"
            class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('sessions.list.more')"
            :aria-label="t('sessions.list.more')"
            @click="openLibraryMenu($event)"
          >
            <EllipsisVertical :size="14" />
          </button>
        </span>
      </header>

      <!--
        What narrows the list: a search over the things that identify a machine, and a filter over
        how a session authenticates. Both live in the store, so the rows, the hit-testing of a drag
        and anything else that asks "what is drawn" get the same answer.
      -->
      <div class="relative mb-2 flex shrink-0 items-center gap-1">
        <div class="relative min-w-0 flex-1">
          <Search
            :size="12"
            class="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground"
          />
          <input
            :value="library.filter.query"
            type="text"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            class="h-7 w-full rounded-md border border-border bg-background pr-6 pl-7 text-xs focus:ring-1 focus:ring-ring focus:outline-none"
            :placeholder="t('sessions.list.search')"
            :aria-label="t('sessions.list.search')"
            @input="library.setQuery(($event.target as HTMLInputElement).value)"
            @keydown.escape.prevent="library.setQuery('')"
          />
          <button
            v-if="library.filter.query !== ''"
            type="button"
            class="absolute top-1/2 right-1 grid size-5 -translate-y-1/2 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('sessions.list.searchClear')"
            :aria-label="t('sessions.list.searchClear')"
            @click="library.setQuery('')"
          >
            <X :size="12" />
          </button>
        </div>
        <button
          ref="filterButton"
          type="button"
          data-session-filter
          class="grid size-7 shrink-0 place-items-center rounded-md border transition-colors"
          :class="
            library.filter.methods.size > 0
              ? 'border-primary/40 bg-primary/10 text-primary'
              : 'border-border text-muted-foreground hover:bg-accent hover:text-foreground'
          "
          :title="t('sessions.list.filter')"
          :aria-label="t('sessions.list.filter')"
          :aria-expanded="filterOpen"
          aria-haspopup="true"
          @click="filterOpen = !filterOpen"
        >
          <ListFilter v-if="library.filter.methods.size > 0" :size="13" />
          <SlidersHorizontal v-else :size="13" />
        </button>

        <!-- The panel hangs off the button that opened it, over the list rather than in it. -->
        <div
          v-if="filterOpen"
          ref="filterBox"
          data-session-filter-panel
          class="absolute top-full right-0 z-30 mt-1 w-52 rounded-lg border border-border bg-card p-1 shadow-xl"
          role="group"
          :aria-label="t('sessions.list.filter')"
        >
          <p class="px-2 pt-1 pb-1 text-[10px] tracking-wide text-muted-foreground uppercase">
            {{ t("sessions.list.filterAuth") }}
          </p>
          <button
            v-for="method in authMethods"
            :key="method.id"
            type="button"
            role="menuitemcheckbox"
            :aria-checked="library.filter.methods.has(method.id)"
            class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
            @click="library.toggleMethod(method.id)"
          >
            <span class="grid size-3 shrink-0 place-items-center text-muted-foreground">
              <Check v-if="library.filter.methods.has(method.id)" :size="12" />
            </span>
            <span class="min-w-0 flex-1 truncate">{{ t(method.labelKey) }}</span>
          </button>
          <button
            v-if="library.filtering"
            type="button"
            class="mt-1 w-full rounded-md border-t border-border px-2 py-1.5 text-left text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="library.clearFilter()"
          >
            {{ t("sessions.list.filterClear") }}
          </button>
        </div>
      </div>

      <p v-if="problem" class="mb-2 shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
        <span class="block text-[11px]">{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>

      <div ref="listBox" data-session-list class="min-h-0 flex-1 overflow-y-auto">
        <!-- A group at the root is named under the header; one made inside a group is named under
             that group's own heading, in the loop below. -->
        <GroupNameForm
          v-if="creatingGroup && creatingInside === null"
          :ref="takeNameForm"
          v-model="newGroupName"
          :icon="FolderPlus"
          :submit-label="t('sessions.list.groupCreate')"
          :busy="groupBusy"
          @submit="createGroup()"
          @cancel="cancelGroupPrompts()"
        />

        <p
          v-if="library.status === 'loading' && library.sessions.length === 0"
          class="px-1 py-2 text-xs text-muted-foreground"
        >
          {{ t("common.status.loading") }}
        </p>

        <div
          v-else-if="library.sessions.length === 0 && library.groups.length === 0"
          class="rounded-md border border-dashed border-border px-3 py-4 text-center"
        >
          <Server :size="18" class="mx-auto mb-2 text-muted-foreground" />
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sessions.list.empty") }}</p>
        </div>

        <!--
          Nothing was found. Drawn as a state of its own rather than as the empty library's line:
          what somebody needs here is to be told the search is why the list is empty, and the way
          back out of it.
        -->
        <div
          v-else-if="library.filtering && library.rows.length === 0"
          class="rounded-md border border-dashed border-border px-3 py-4 text-center"
        >
          <Search :size="16" class="mx-auto mb-2 text-muted-foreground" />
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sessions.list.noMatches") }}</p>
          <button
            type="button"
            class="mt-2 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
            @click="library.clearFilter()"
          >
            {{ t("sessions.list.filterClear") }}
          </button>
        </div>

        <template v-else>
          <template v-for="row in library.rows" :key="row.key">
            <!-- A group: the heading, the two drawers for changing it, and the place it is dropped
                 into. The whole row is the drag handle; the chevron opens and closes it. -->
            <div
              v-if="row.kind === 'group'"
              :data-row="row.key"
              class="relative flex items-center gap-1 rounded-md py-1 pr-1 transition-colors hover:bg-accent/40"
              :class="markOf(row.key) === 'ring' ? 'bg-primary/10 ring-1 ring-primary/50' : ''"
              :style="indent(row.depth)"
              @contextmenu="openHeadingMenu($event, row.group)"
              @mousedown="startDrag($event, { kind: 'group', id: row.id }, row.group.name)"
            >
              <span
                v-if="markOf(row.key) === 'line-before'"
                class="absolute inset-x-1 top-0 h-0.5 rounded bg-primary"
                aria-hidden="true"
              ></span>
              <span
                v-if="markOf(row.key) === 'line-after'"
                class="absolute inset-x-1 bottom-0 h-0.5 rounded bg-primary"
                aria-hidden="true"
              ></span>
              <button
                type="button"
                class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:text-foreground"
                :title="
                  t(row.collapsed ? 'sessions.list.groupExpand' : 'sessions.list.groupCollapse', {
                    name: row.group.name,
                  })
                "
                :aria-label="
                  t(row.collapsed ? 'sessions.list.groupExpand' : 'sessions.list.groupCollapse', {
                    name: row.group.name,
                  })
                "
                :aria-expanded="!row.collapsed"
                @click="library.setCollapsed(row.id, !row.collapsed)"
              >
                <ChevronRight :size="12" :class="row.collapsed ? '' : 'rotate-90'" />
              </button>

              <!-- A folder, drawn like the file browser's: the same shape means the same thing
                   wherever a tree is nested. -->
              <FolderOpen v-if="!row.collapsed" :size="14" class="shrink-0 text-primary" />
              <Folder v-else :size="14" class="shrink-0 text-primary" />

              <input
                v-if="renamingGroup === row.id"
                :ref="takeRenameField"
                :value="groupNameText"
                type="text"
                class="min-w-0 flex-1 rounded-sm border border-border bg-background px-1 py-0.5 text-xs focus:outline-none"
                :aria-label="t('sessions.list.groupName')"
                @input="groupNameText = ($event.target as HTMLInputElement).value"
                @keydown.enter.prevent="submitGroupRename(row.group)"
                @keydown.escape.prevent="cancelGroupPrompts()"
                @blur="submitGroupRename(row.group)"
              />
              <h3 v-else class="min-w-0 flex-1 truncate text-xs font-medium">{{ row.group.name }}</h3>
            </div>

            <GroupNameForm
              v-if="row.kind === 'group' && creatingGroup && creatingInside === row.id"
              :ref="takeNameForm"
              v-model="newGroupName"
              :icon="FolderPlus"
              :submit-label="t('sessions.list.groupCreate')"
              :busy="groupBusy"
              @submit="createGroup()"
              @cancel="cancelGroupPrompts()"
            />

            <!-- A session: one row, indented by how deep its group is. -->
            <template v-if="row.kind === 'session'">
              <div
                :data-row="row.key"
                class="group relative flex items-center gap-0.5 rounded-md pr-1 transition-colors hover:bg-accent/60"
                :class="markOf(row.key) === 'ring' ? 'bg-primary/10 ring-1 ring-primary/50' : ''"
                :style="indent(row.depth)"
                @contextmenu="openSessionMenu($event, row.session)"
                @mousedown="startDrag($event, { kind: 'session', id: row.session.id }, row.session.name)"
              >
                <button
                  type="button"
                  class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left"
                  :title="t('sessions.list.open', { name: row.session.name })"
                  @click="terminals.open(row.session)"
                >
                  <Terminal :size="13" class="shrink-0 text-muted-foreground" />
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-sm">{{ row.session.name }}</span>
                    <span class="block truncate font-mono text-[10px] text-muted-foreground">{{
                      endpoint(row.session)
                    }}</span>
                  </span>
                  <LoaderCircle
                    v-if="terminals.connectingSessionIds.includes(row.session.id)"
                    :size="12"
                    class="shrink-0 animate-spin text-muted-foreground"
                    :aria-label="t('sessions.list.connecting', { name: row.session.name })"
                  />
                </button>

                <button
                  type="button"
                  class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('files.open', { name: row.session.name })"
                  :aria-label="t('files.open', { name: row.session.name })"
                  @click="terminals.openFiles(row.session)"
                >
                  <Folder :size="12" />
                </button>
                <button
                  type="button"
                  class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('sessions.list.edit')"
                  :aria-label="t('sessions.list.edit')"
                  @click="startEdit(row.session)"
                >
                  <Pencil :size="12" />
                </button>
                <button
                  type="button"
                  class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
                  :title="t('sessions.list.remove')"
                  :aria-label="t('sessions.list.remove')"
                  @click="pendingRemoval = { kind: 'session', session: row.session }"
                >
                  <Trash2 :size="12" />
                </button>
              </div>
            </template>

            <!--
              A group with nothing in it says so, once, where its contents would have been. Sessions
              that are in no group need no such line: they are drawn at the root like anything else
              that is not inside a folder, and the list of them says what it is.
            -->
            <p
              v-if="row.kind === 'hint'"
              class="px-1 py-0.5 text-[10px] text-muted-foreground"
              :style="indent(row.depth)"
            >
              {{ t("sessions.list.groupEmpty") }}
            </p>
          </template>

          <!--
            Where a dragged session goes to leave every group. It is drawn only while one is in the
            air, at the end of the list rather than at its top so that nothing already drawn moves
            under the pointer: what it replaces is the ungrouped heading, which was there the rest of
            the time to say something the rows say themselves.
          -->
          <div
            v-if="dropOutRow"
            :data-row="DROP_OUT_ROW"
            class="mt-1 flex items-center gap-2 rounded-md border border-dashed px-2 py-1.5 text-[11px] transition-colors"
            :class="
              markOf(DROP_OUT_ROW) === 'ring'
                ? 'border-primary bg-primary/10 text-foreground'
                : 'border-border text-muted-foreground'
            "
          >
            <FolderInput :size="13" class="shrink-0" />
            {{ t("sessions.list.dropOutOfGroups") }}
          </div>
        </template>
      </div>

      <!-- What is being dragged, drawn under the pointer: a picture of the row that is moving. -->
      <div
        v-if="dragGhost"
        data-drag-ghost
        class="pointer-events-none fixed z-50 max-w-48 truncate rounded-md border border-border bg-card px-2 py-1 text-[11px] shadow-md"
        :style="{ left: `${dragGhost.x + 12}px`, top: `${dragGhost.y + 8}px` }"
      >
        {{ dragGhost.label }}
      </div>
    </section>

    <section v-if="hasTunnels" class="mt-4 shrink-0 px-3">
      <h2 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("terminal.tunnels.title") }}
      </h2>

      <p v-if="tunnelProblem" class="mb-2 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
        <span class="block text-[11px]">{{ tunnelProblem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ tunnelProblem.code }}</span>
        <button
          type="button"
          class="mt-1 rounded-sm text-[10px] text-muted-foreground transition-colors hover:text-foreground"
          @click="terminals.dismissTunnelProblem()"
        >
          {{ t("terminal.tunnels.dismiss") }}
        </button>
      </p>

      <ul v-if="terminals.tunnels.length > 0" class="max-h-44 space-y-0.5 overflow-y-auto">
        <li
          v-for="tunnel in terminals.tunnels"
          :key="tunnel.id"
          class="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-accent/60"
          @contextmenu="openTunnelMenu($event, tunnel)"
        >
          <Network :size="13" class="shrink-0 text-muted-foreground" />
          <span class="min-w-0 flex-1">
            <span class="flex items-center gap-1 font-mono text-[11px]">
              <span class="min-w-0 truncate">{{ tunnel.bindHost }}:{{ tunnel.bindPort }}</span>
              <ArrowRight
                :size="11"
                class="shrink-0 text-muted-foreground"
                role="img"
                :aria-label="t('terminal.tunnels.direction')"
              />
              <span v-if="tunnel.mode === 'dynamic'" class="min-w-0 truncate font-mono text-[11px]">SOCKS5</span>
              <span v-else class="min-w-0 truncate">{{ tunnel.targetHost }}:{{ tunnel.targetPort }}</span>
            </span>
            <span class="block text-[10px] text-muted-foreground">
              {{ t("terminal.tunnels.connections", { count: tunnel.connections }, tunnel.connections) }}
            </span>
          </span>
          <button
            type="button"
            class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
            :title="t('terminal.tunnels.stop')"
            :aria-label="t('terminal.tunnels.stop')"
            @click="terminals.stopTunnel(tunnel.id)"
          >
            <X :size="12" />
          </button>
        </li>
      </ul>

      <ul v-if="terminals.tunnelFailures.length > 0" class="mt-1 max-h-44 space-y-1 overflow-y-auto">
        <li
          v-for="failure in terminals.tunnelFailures"
          :key="tunnelFailureKey(failure)"
          class="rounded-md border border-warning/40 bg-warning/10 px-2 py-1.5"
        >
          <div class="flex items-start gap-1.5">
            <TriangleAlert :size="12" class="mt-0.5 shrink-0 text-warning" />
            <span class="min-w-0 flex-1">
              <span class="block text-[11px]">
                {{ t("terminal.tunnels.failure", { bind: `${failure.bindHost}:${failure.bindPort}` }) }}
              </span>
              <span class="mt-0.5 block font-mono text-[10px] break-all text-muted-foreground">{{
                failure.detail
              }}</span>
            </span>
            <button
              type="button"
              class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
              :title="t('terminal.tunnels.dismiss')"
              :aria-label="t('terminal.tunnels.dismiss')"
              @click="terminals.dismissTunnelFailure(tunnelFailureKey(failure))"
            >
              <X :size="11" />
            </button>
          </div>
        </li>
      </ul>
    </section>

    <section class="mt-4 shrink-0 px-3">
      <h2 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("shell.sidebar.protocols") }}
      </h2>
      <ul class="space-y-0.5">
        <li
          v-for="protocol in protocols"
          :key="protocol.id"
          class="flex items-center justify-between rounded-md px-2 py-1.5 text-sm hover:bg-accent/60"
        >
          <span class="flex items-center gap-2">
            <Terminal :size="14" class="text-muted-foreground" />
            {{ t(protocol.labelKey) }}
          </span>
          <span class="font-mono text-[10px] text-muted-foreground">{{ protocol.defaultPort ?? "—" }}</span>
        </li>
      </ul>
    </section>

    <div class="mt-auto flex items-center gap-2 px-4 py-3 text-[11px] text-muted-foreground">
      <span class="size-1.5 rounded-full" :class="statusDot"></span>
      {{ statusLabel }}
    </div>

    <SessionDialog :open="editorOpen" :session="editing" @update:open="editorOpen = $event" />

    <!-- What the export file will hold, asked once, before a file is asked for. -->
    <ExportSessionsDialog
      :open="exportOpen"
      :busy="exportBusy"
      @confirm="runExport($event)"
      @update:open="exportOpen = $event"
    />

    <!--
      One question for both kinds of removal, drawn over everything rather than inside the list: a
      row that opens a strip of its own puts the question under an indentation and a scroll position
      that have nothing to do with it, and lets two questions be open at once.
    -->
    <ConfirmDialog
      v-if="removal"
      :open="true"
      :title="removal.title"
      :message="removal.message"
      :confirm-label="removal.title"
      danger
      @confirm="confirmRemoval()"
      @update:open="pendingRemoval = null"
    />
  </aside>
</template>
