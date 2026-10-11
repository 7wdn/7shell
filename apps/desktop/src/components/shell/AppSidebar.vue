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
 * separate buttons, and deleting always asks first, in the row, before anything is removed. The
 * editor dialog those buttons raise is mounted by the frame rather than here: the home page opens it
 * too, and the sidebar is not always on screen. See `App.vue`.
 *
 * The tunnels section draws what the terminals store was told by the backend, and only then: an
 * empty section is not shown at all. A forward that failed to come up is shown next to the ones
 * that are, because "the tunnel is not there" is the answer somebody staring at a port needs.
 */
import {
  ArrowDownUp,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsLeft,
  Download,
  Minus,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  ListFilter,
  LoaderCircle,
  LocateFixed,
  Network,
  Pin,
  Plus,
  RefreshCw,
  Search,
  Server,
  RotateCcw,
  Square,
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
import SessionDetails from "@/components/sessions/SessionDetails.vue";
import SessionFailure from "@/components/sessions/SessionFailure.vue";
import SidebarResizeHandle from "@/components/shell/SidebarResizeHandle.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import SystemIcon from "@/components/ui/SystemIcon.vue";
import { HoverCardRoot, HoverCardTrigger } from "reka-ui";

import { useContextMenu } from "@/composables/useContextMenu";
import { useToast } from "@/composables/useToast";
import { useTreeDrag } from "@/composables/useTreeDrag";
import { describeShellError } from "@/i18n/errors";
import { isTextEntry } from "@/lib/menu/editable";
import { systemLabel, systemMark } from "@/lib/system/icons";
import { groupMenu, sessionMenu, tunnelMenu } from "@/lib/menu/sessions";
import { MENU_TITLES } from "@/lib/menu/titles";
import type { MenuEntry } from "@/lib/menu/types";
import { groupDropFor, sessionDropFor } from "@/lib/sessions/drag";
import { rememberExportPassphrase } from "@/lib/sessions/exportPassphrase";
import { regexIsInvalid } from "@/lib/sessions/filter";
import { drawOrder, groupPath, hasSibling } from "@/lib/sessions/groups";
import { importReceipt } from "@/lib/sessions/receipt";
import { rowTarget } from "@/lib/sessions/tree";
import { usePreferencesStore } from "@/stores/preferences";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { sessionOf, tunnelFailureKey, useTerminalsStore } from "@/stores/terminals";
import type { ShellError } from "@/lib/backend/errors";
import type { OpenTunnel, Session, SessionGroup } from "@/types/session";
import { AUTH_METHODS } from "@/types/generated/protocols";

const { t } = useI18n();
const library = useSessionsStore();
const preferences = usePreferencesStore();
const editor = useSessionEditorStore();
const terminals = useTerminalsStore();
const { openMenu } = useContextMenu();
const { toast } = useToast();

const emit = defineEmits<{
  /** Put the sidebar away: the frame around this panel owns where it goes and how it comes back. */
  collapse: [];
}>();

/** The ways a saved session can authenticate, which is what the list can be narrowed by. */
const authMethods = computed(() => AUTH_METHODS);

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

/**
 * The name being typed over a group's own, and which group that is.
 *
 * One field for both cases, because they are the same case: a group that was just made is one whose
 * name is still the placeholder, and a name somebody wants to change is a name somebody wants to
 * type. Nothing is written until the typing stops, and an empty name, or the one it already had, is
 * a cancel — the field is not a form, it is the name itself.
 */
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
  editor.startNew();
}

function startEdit(session: Session): void {
  editor.startEdit(session);
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

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", closeFilterOnOutsideClick, true);
  document.removeEventListener("keydown", exitSelectionOnEscape, true);
  window.clearTimeout(flashTimer);
});

/**
 * Reads a file the user picks and says what it added. A refusal stays in the list's own box.
 *
 * A refusal being a *list* failure is what the second half of this does the rest of the way: the
 * import writes the library, so what went wrong belongs where the library is drawn rather than on
 * the button that started it. The home page's shortcut is the one that cannot rely on that — see
 * `HomeView`.
 */
async function runImport(): Promise<void> {
  const summary = await library.importLibrary(t("sessions.list.importTitle"));
  if (!summary) {
    return;
  }
  toast(importReceipt(summary, t), summary.missingKeys + summary.droppedHops > 0 ? "result" : "confirmation");
}

/**
 * Writes the library to a file the user picks.
 *
 * `passphrase` is the whole of what was decided: with one the file carries the stored logins sealed,
 * with `null` it carries none. The receipt says which of the two happened, because the two files go
 * to different places and one of them needs the passphrase again on the other side.
 *
 * A passphrase is remembered only once a file has actually been written with it, so the next export
 * in this sitting does not ask for it twice. A dismissed file dialog is not a passphrase that
 * worked, and an export that carried nothing has nothing worth remembering.
 */
async function runExport(passphrase: string | null): Promise<void> {
  exportBusy.value = true;
  const summary = await library.exportLibrary(t("sessions.list.exportTitle"), SESSION_FILE_NAME, passphrase);
  exportBusy.value = false;
  // Closed either way: a dismissed file dialog and a failed write are both "nothing more to decide
  // here", and a failure is already drawn in the list's own box.
  exportOpen.value = false;
  if (summary) {
    if (passphrase !== null) {
      rememberExportPassphrase(passphrase);
    }
    toast(
      t(
        summary.encrypted ? "sessions.list.exportDoneEncrypted" : "sessions.list.exportDone",
        { count: summary.sessions, file: summary.fileName },
        summary.sessions,
      ),
      "receipt",
    );
  }
}

/** What the import/export button opens: the two things done to the whole list rather than to a row. */
function openTransferMenu(event: MouseEvent): void {
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

/** Whether the query is a pattern that does not compile, which the box says by turning red. */
const regexInvalid = computed(() => library.filter.regex && regexIsInvalid(library.filter.query));

/** How many sessions are open right now, which is what the active-only button reports. */
const openCount = computed(() => library.openSessionIds.size);

/** The row that has just been located, drawn for a moment so an eye can find it. */
const flashedRow = ref<string | null>(null);
let flashTimer = 0;

/**
 * Draws one row for a moment and then leaves it alone.
 *
 * Cleared before it is set again, so asking twice draws twice: a class that is already on the row
 * is a class the browser has nothing to do about, and the second ask would look like nothing.
 */
async function flash(key: string): Promise<void> {
  window.clearTimeout(flashTimer);
  flashedRow.value = null;
  await nextTick();
  flashedRow.value = key;
  flashTimer = window.setTimeout(() => {
    flashedRow.value = null;
  }, 1400);
}

/**
 * Finds the session the window is showing, in the list.
 *
 * The one thing the list cannot answer on its own: a session can be open — a terminal, a file
 * browser, a page about its host — while its row is off screen, behind a closed folder, or behind a
 * search somebody has since typed. So the search and the switches are cleared first, which is what
 * makes the row exist at all, and then it is brought into view and marked.
 *
 * Nothing is open on the home page, so there is nothing to find and the button says so by being
 * disabled rather than by doing nothing.
 */
async function locateActive(): Promise<void> {
  const session = terminals.activeTab ? sessionOf(terminals.activeTab) : null;
  if (!session) {
    return;
  }
  library.clearFilter();
  await nextTick();
  const row = rowForSession(session.id);
  if (!row) {
    return;
  }
  row.scrollIntoView({ block: "center" });
  await flash(`session:${session.id}`);
}

/** The row a session is drawn in, once it is on screen. */
function rowForSession(sessionId: string): HTMLElement | null {
  return listBox.value?.querySelector<HTMLElement>(`[data-row="session:${sessionId}"]`) ?? null;
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

/**
 * Makes a group and puts its name in front of the person to type over.
 *
 * One click rather than a form, because a group is a heading and the only thing anybody wants to
 * decide about one is what it says: the name is decided where the name is written — in the row
 * itself, through the same field that renames one — instead of in a form that has to be typed into
 * and then submitted before anything exists. The name it starts with is the one the form used to
 * ask for, so somebody who changes their mind about naming it still has a group rather than a
 * half-filled form to throw away.
 *
 * A group made inside a closed one opens it, and the row it landed in is brought into view: a group
 * that was just made has to be visible if it is to be named, and the end of a long list is exactly
 * where the new one is.
 *
 * A list that is narrowed is let out of it first. The name is typed in the row, so a group the
 * search is hiding is a group that cannot be named — and "New group" doing nothing visible is worse
 * than a search being cleared by the person who asked for a new group.
 */
async function startGroupCreate(parentId: string | null): Promise<void> {
  if (groupBusy.value) {
    return;
  }
  library.clearFilter();
  groupBusy.value = true;
  const created = await library.addGroup(t("sessions.list.groupNewDefault"), parentId);
  groupBusy.value = false;
  if (!created) {
    return;
  }
  startGroupRename(created);
  await nextTick();
  rowFor(created.id)?.scrollIntoView({ block: "nearest" });
}

/** The row a group is drawn in, once it is on screen. */
function rowFor(groupId: string): HTMLElement | null {
  return listBox.value?.querySelector<HTMLElement>(`[data-row="group:${groupId}"]`) ?? null;
}

/**
 * A group made for a session, with the session put inside it.
 *
 * The two writes are one action to the person who asked for it, so a refusal of the first is where
 * it stops: a group with a session missing from it would be a folder somebody has to find out about
 * afterwards. The name is typed over in the row afterwards, exactly as it is for a group made from
 * the header — and, for the same reason, a narrowed list is let out of it first: the session has
 * just moved into the new group, and both of them have to be on screen.
 */
async function newGroupForSession(session: Session): Promise<void> {
  if (groupBusy.value) {
    return;
  }
  library.clearFilter();
  groupBusy.value = true;
  const created = await library.addGroup(t("sessions.list.groupNewDefault"), null);
  if (created) {
    await library.placeSession(session.id, { kind: "group", id: created.id });
  }
  groupBusy.value = false;
  if (!created) {
    return;
  }
  startGroupRename(created);
  await nextTick();
  rowFor(created.id)?.scrollIntoView({ block: "nearest" });
}

function startGroupRename(group: SessionGroup): void {
  groupNameText.value = group.name;
  renamingGroup.value = group.id;
  void nextTick().then(() => {
    renameField.value?.focus();
    renameField.value?.select();
  });
}

/** Closes the name being typed, whatever it was doing there. */
function cancelGroupRename(): void {
  renamingGroup.value = null;
}

/**
 * The rename field hands itself over through here: a `ref` inside a list arrives as an array of
 * everything the list rendered, and there is only ever one field on screen.
 */
function takeRenameField(field: unknown): void {
  renameField.value = (field as HTMLInputElement | null) ?? null;
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

/** Whether an attempt for this session is in flight, which the row draws around. */
function isConnecting(session: Session): boolean {
  return terminals.connectingSessionIds.includes(session.id);
}

/** What the last attempt at this session failed with, or nothing when it did not. */
function failureOf(session: Session): ShellError | undefined {
  return terminals.failures[session.id];
}

/**
 * Whether this session's host said something this build has a mark for.
 *
 * Not the same as having answered: a host can say "OpenBSD" quite clearly and still have no mark
 * here, and that row keeps the plain terminal — a mark nobody drew is not an answer, and borrowing
 * another system's would be worse than drawing none.
 */
function hasMark(session: Session): boolean {
  return systemMark(terminals.systemOf(session)) !== null;
}

/** What to call that mark out loud, for the row a reader meets with a screen reader. */
function markTitle(session: Session): string | null {
  const name = systemLabel(terminals.systemOf(session));
  return name ? t("sessions.list.system", { name }) : null;
}

function isSelected(id: string): boolean {
  return library.selected.has(id);
}

/**
 * The tick on a group: everything under it, some of it, or none of it.
 *
 * A group is not a thing that can be in a selection — what the bulk actions act on are sessions —
 * so the group's own box is a statement about what is under it, and ticking it picks all of them.
 */
function groupPicked(group: SessionGroup): "none" | "partial" | "all" {
  const ids = library.sessionsUnder(group.id);
  if (ids.length === 0) {
    return "none";
  }
  const picked = ids.filter((id) => library.selected.has(id)).length;
  return picked === 0 ? "none" : picked === ids.length ? "all" : "partial";
}

function pickGroup(group: SessionGroup): void {
  const ids = library.sessionsUnder(group.id);
  if (groupPicked(group) === "all") {
    const remaining = [...library.selected].filter((id) => !ids.includes(id));
    library.setSelected(remaining);
    return;
  }
  library.setSelected([...new Set([...library.selected, ...ids])]);
  // An opened group is one whose contents can be seen to be picked.
  library.setCollapsed(group.id, false);
}

/** Where a shift-click measures from: the last row that was picked or opened. */
const selectionAnchor = ref<string | null>(null);

/**
 * Picking one row out, with the two modifiers a list is expected to answer.
 *
 * Shift extends from the last row that was touched, over the rows as they are drawn — the drawn
 * order is the only order a person can see, and a range that did not follow it would be a range
 * nobody could aim.
 */
function pickSession(event: MouseEvent, session: Session): void {
  if (event.shiftKey && selectionAnchor.value !== null) {
    const visible = library.rows
      .filter((row) => row.kind === "session")
      .map((row) => (row.kind === "session" ? row.session.id : ""));
    const from = visible.indexOf(selectionAnchor.value);
    const to = visible.indexOf(session.id);
    if (from >= 0 && to >= 0) {
      const [low, high] = from <= to ? [from, to] : [to, from];
      library.setSelected([...new Set([...library.selected, ...visible.slice(low, high + 1)])]);
      return;
    }
  }
  library.toggleSelected(session.id);
  selectionAnchor.value = session.id;
}

/** Every session the list is showing, which is what the select-all control is about. */
const visibleSessionIds = computed(() =>
  library.rows.filter((row) => row.kind === "session").map((row) => (row.kind === "session" ? row.session.id : "")),
);

/** Whether every one of them is picked, which is what the tri-state control reports. */
const allSessionsPicked = computed(
  () => visibleSessionIds.value.length > 0 && visibleSessionIds.value.every((id) => library.selected.has(id)),
);

const selectAllLabel = computed(() =>
  allSessionsPicked.value ? t("sessions.list.deselectAll") : t("sessions.list.selectAll"),
);

/** Picks every session in the list, or puts every one back. */
function toggleAllSessions(): void {
  const all = library.rows
    .filter((row) => row.kind === "session")
    .map((row) => (row.kind === "session" ? row.session.id : ""));
  const picked = all.filter((id) => library.selected.has(id)).length;
  library.setSelected(picked === all.length ? [] : all);
}

async function placeSelected(groupId: string | undefined): Promise<void> {
  await library.placeSelected(groupId);
}

/** Deletes everything picked, after the one question a delete asks. */
const deleteSelectedOpen = ref(false);

async function confirmDeleteSelected(): Promise<void> {
  deleteSelectedOpen.value = false;
  await library.removeSelected();
}

/** The names of what is picked, for the question that names them. */
const selectedNames = computed(() =>
  library.sessions.filter((session) => library.selected.has(session.id)).map((session) => session.name),
);

/** A group made for the picked sessions, with all of them moved into it and the name typed over. */
async function newGroupForSelection(): Promise<void> {
  if (groupBusy.value || library.selected.size === 0) {
    return;
  }
  library.clearFilter();
  groupBusy.value = true;
  const created = await library.addGroup(t("sessions.list.groupNewDefault"), null);
  if (created) {
    await library.placeSelected(created.id);
  }
  groupBusy.value = false;
  if (!created) {
    return;
  }
  startGroupRename(created);
  await nextTick();
  rowFor(created.id)?.scrollIntoView({ block: "nearest" });
}

/** Where the picked sessions can be sent, as the same list the row's own menu offers. */
function openMoveSelectedMenu(event: MouseEvent): void {
  const entries: MenuEntry[] = drawOrder(library.groups).map(({ group }) => ({
    kind: "action",
    id: `move-selected-to-${group.id}`,
    labelKey: "sessions.list.groupLabel",
    params: { name: groupPath(library.groups, group.id) },
    checked: false,
    run: () => void placeSelected(group.id),
  }));
  if (entries.length > 0) {
    entries.unshift({
      kind: "action",
      id: "move-selected-to-ungrouped",
      labelKey: "sessions.editor.groupNone",
      checked: false,
      run: () => void placeSelected(undefined),
    });
  }
  openMenu(event, { key: MENU_TITLES.session }, entries);
}

/**
 * Escape leaves the selection, wherever the keyboard is.
 *
 * A listener on the document rather than on the panel, because the rows are not focusable: the
 * keyboard is usually in a terminal or a search box when somebody decides they are done picking
 * things, and a key that only works when the pointer happens to be over the list is a key nobody
 * can rely on. Text fields are left alone — Escape belongs to whatever is being typed.
 */
function exitSelectionOnEscape(event: KeyboardEvent): void {
  if (event.key !== "Escape" || !library.selecting || isTextEntry(event.target)) {
    return;
  }
  library.clearSelection();
}

watch(
  () => library.selecting,
  (selecting) => {
    if (selecting) {
      document.addEventListener("keydown", exitSelectionOnEscape, true);
    } else {
      document.removeEventListener("keydown", exitSelectionOnEscape, true);
    }
  },
);

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
      newGroup: (target) => void newGroupForSession(target),
    }),
  );
}

/**
 * The sessions inside a group that something is open for, its subgroups included.
 *
 * Which is what the heading's closing entry is about. The saved sessions are not touched by it:
 * what ends is this run's connections to them.
 */
function openInside(group: SessionGroup): string[] {
  return library.sessionsUnder(group.id).filter((id) => library.openSessionIds.has(id));
}

/** A group heading's own menu: where it sits, what it is called, and what is open inside it. */
function openHeadingMenu(event: MouseEvent, group: SessionGroup): void {
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
        pin: (target, pinned) => void library.setGroupPinned(target.id, pinned),
        // Asked again rather than taken from the count the menu drew with: what closes is what is
        // open now, and a moment-old number would be a promise about a set that just changed.
        close: (target) => void terminals.closeSessions(openInside(target)),
      },
      {
        up: hasSibling(library.groups, group.id, -1),
        down: hasSibling(library.groups, group.id, 1),
        open: openInside(group).length,
      },
    ),
  );
}

function openTunnelMenu(event: MouseEvent, tunnel: OpenTunnel): void {
  openMenu(event, { key: MENU_TITLES.tunnel }, tunnelMenu(tunnel, { stop: (id) => void terminals.stopTunnel(id) }));
}
</script>

<template>
  <aside
    data-panel-frame
    class="relative flex shrink-0 flex-col border-r border-border bg-card/30"
    :style="{ width: `${preferences.resolvedSidebarWidth}px` }"
  >
    <section class="flex min-h-0 flex-1 flex-col px-3 pt-3">
      <!--
        The row the panel is operated from: bringing the library in or sending it out on the left —
        the one thing that is about the whole list rather than a row, and the place a word about the
        panel would otherwise go — and the controls that act on the list on the right.
      -->
      <header class="mb-2 flex shrink-0 items-center gap-1">
        <!--
          Picking rows out replaces the panel's own controls with the ones that act on what is
          picked: a toolbar that still offered "new session" while a selection is being built would
          be answering a question nobody asked, and the count is the thing to look at instead.
        -->
        <template v-if="library.selecting">
          <span class="min-w-0 truncate text-xs font-medium text-muted-foreground">
            {{ t("shell.sidebar.sessions") }}
          </span>
          <span class="shrink-0 text-[11px] font-normal text-muted-foreground/80" data-session-selected-count>
            {{ t("sessions.list.selected", { count: library.selected.size }, library.selected.size) }}
          </span>
          <span class="min-w-0 flex-1"></span>
          <span class="flex shrink-0 items-center gap-0.5">
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('sessions.list.addGroup')"
              :aria-label="t('sessions.list.addGroup')"
              @click="newGroupForSelection()"
            >
              <FolderPlus :size="12" />
            </button>
            <button
              type="button"
              data-session-move-selected
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('menu.moveToGroup')"
              :aria-label="t('menu.moveToGroup')"
              aria-haspopup="true"
              @click="openMoveSelectedMenu($event)"
            >
              <FolderInput :size="12" />
            </button>
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
              :title="t('sessions.list.removeSelected', { count: library.selected.size }, library.selected.size)"
              :aria-label="t('sessions.list.removeSelected', { count: library.selected.size }, library.selected.size)"
              @click="deleteSelectedOpen = true"
            >
              <Trash2 :size="12" />
            </button>
            <button
              type="button"
              data-session-select-all
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="selectAllLabel"
              :aria-label="selectAllLabel"
              @click="toggleAllSessions()"
            >
              <Check v-if="allSessionsPicked" :size="12" class="text-primary" />
              <Minus v-else-if="library.selected.size > 0" :size="12" class="text-primary" />
              <Square v-else :size="12" class="stroke-[1.7]" />
            </button>
            <button
              type="button"
              data-session-exit-selection
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('sessions.list.exitSelection')"
              :aria-label="t('sessions.list.exitSelection')"
              @click="library.clearSelection()"
            >
              <X :size="12" />
            </button>
          </span>
        </template>
        <template v-else>
          <button
            type="button"
            data-session-transfer
            class="inline-flex h-7 min-w-0 items-center gap-1 rounded-md px-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('sessions.list.importExport')"
            :aria-label="t('sessions.list.importExport')"
            aria-haspopup="true"
            @click="openTransferMenu($event)"
          >
            <ArrowDownUp :size="13" class="shrink-0" />
            <span class="min-w-0 truncate">{{ t("sessions.list.importExport") }}</span>
            <ChevronDown :size="12" class="shrink-0 opacity-60" />
          </button>

          <span class="min-w-0 flex-1"></span>

          <span class="flex shrink-0 items-center gap-0.5">
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('sessions.list.collapseAll')"
              :aria-label="t('sessions.list.collapseAll')"
              @click="library.collapseAll()"
            >
              <ChevronsDownUp :size="12" />
            </button>
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('sessions.list.addGroup')"
              :aria-label="t('sessions.list.addGroup')"
              @click="startGroupCreate(null)"
            >
              <FolderPlus :size="12" />
            </button>
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('sessions.list.add')"
              :aria-label="t('sessions.list.add')"
              @click="startNew()"
            >
              <Plus :size="12" />
            </button>
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-60"
              :title="t('sessions.list.refresh')"
              :aria-label="t('sessions.list.refresh')"
              :disabled="library.status === 'loading'"
              @click="library.load()"
            >
              <RefreshCw :size="12" :class="library.status === 'loading' ? 'animate-spin' : ''" />
            </button>
            <button
              type="button"
              class="grid size-5 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :title="t('shell.sidebar.collapse')"
              :aria-label="t('shell.sidebar.collapse')"
              @click="emit('collapse')"
            >
              <ChevronsLeft :size="12" />
            </button>
          </span>
        </template>
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
            class="h-7 w-full rounded-md border bg-background pr-12 pl-7 text-xs select-text focus:ring-1 focus:ring-ring focus:outline-none"
            :class="regexInvalid ? 'border-destructive focus:ring-destructive' : 'border-border'"
            :aria-invalid="regexInvalid ? 'true' : 'false'"
            :placeholder="t('sessions.list.search')"
            :aria-label="t('sessions.list.search')"
            @input="library.setQuery(($event.target as HTMLInputElement).value)"
            @keydown.escape.prevent="library.setQuery('')"
          />
          <!--
            How the query is read, said next to where it is typed: words to look for, or one regular
            expression over the same text. The glyph is the pattern language's own shorthand for
            "any text", which is what somebody reaching for this already has in mind.
          -->
          <button
            type="button"
            data-session-regex
            class="absolute top-1/2 right-7 grid h-5 min-w-5 -translate-y-1/2 place-items-center rounded-sm px-0.5 font-mono text-[10px] transition-colors"
            :class="
              library.filter.regex
                ? 'bg-primary/10 text-primary'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground'
            "
            :title="t('sessions.list.regexHint')"
            :aria-label="t('sessions.list.regex')"
            :aria-pressed="library.filter.regex"
            @click="library.toggleRegex()"
          >
            .*
          </button>
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

        <!-- Find what is on screen: the one thing the list cannot work out on its own. -->
        <button
          type="button"
          data-session-locate
          class="grid size-7 shrink-0 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50 disabled:hover:bg-transparent"
          :title="t('sessions.list.locate')"
          :aria-label="t('sessions.list.locate')"
          :disabled="terminals.activeTab === null || sessionOf(terminals.activeTab) === null"
          @click="locateActive()"
        >
          <LocateFixed :size="13" />
        </button>
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
        <button
          type="button"
          data-session-active-only
          class="relative grid size-7 shrink-0 place-items-center rounded-md border transition-colors"
          :class="
            library.filter.onlyOpen
              ? 'border-primary/40 bg-primary/10 text-primary'
              : 'border-border text-muted-foreground hover:bg-accent hover:text-foreground'
          "
          :title="t('sessions.list.onlyActive')"
          :aria-label="t('sessions.list.onlyActive')"
          :aria-pressed="library.filter.onlyOpen"
          @click="library.toggleOnlyOpen()"
        >
          <span
            class="pointer-events-none absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full border-[1.5px] transition-all duration-200 ease-out"
            :class="openCount > 0 ? 'size-1.5 border-success bg-success' : 'size-3 border-current'"
            aria-hidden="true"
          ></span>
          <span class="sr-only" aria-live="polite">{{ t("sessions.list.activeCount", { count: openCount }) }}</span>
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
            v-if="library.filter.methods.size > 0"
            type="button"
            class="mt-1 flex w-full items-center gap-2 rounded-md border-t border-border px-2 py-1.5 text-left text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="library.clearMethods()"
          >
            <RotateCcw :size="12" class="shrink-0" />
            <span class="min-w-0 flex-1 truncate">{{ t("sessions.list.filterClear") }}</span>
          </button>
        </div>
      </div>

      <p v-if="problem" class="mb-2 shrink-0 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
        <span class="block text-[11px]">{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>

      <!--
        Nothing in the panel is text to be selected. What a drag across these rows means is a drag —
        carrying a session somewhere, or picking several out — and a list that turns blue under the
        pointer is a list fighting the hand that is using it. The names can still be copied, from
        the menu that offers exactly that.
      -->
      <div
        ref="listBox"
        data-session-list
        class="min-h-0 flex-1 overflow-y-auto select-none"
        @click="library.clearSelection()"
      >
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
          A search that matched nothing draws nothing at all. Not a sentence, not a button: the
          person can see what they typed, the list is empty because of it, and the way back — the
          box with the cross in it — is the thing they are already looking at.
        -->
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
                @click.stop="library.setCollapsed(row.id, !row.collapsed)"
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
                class="min-w-0 flex-1 rounded-sm border border-border bg-background px-1 py-0.5 text-sm select-text focus:outline-none"
                :aria-label="t('sessions.list.groupName')"
                @input="groupNameText = ($event.target as HTMLInputElement).value"
                @keydown.enter.prevent="submitGroupRename(row.group)"
                @keydown.escape.prevent="cancelGroupRename()"
                @blur="submitGroupRename(row.group)"
              />
              <!-- The heading is set at the size the rows are set at — a group is a folder in the
                   same list, not a title over it — and the folder and the chevron are what say it
                   is one. The reference draws a group's name with the very same class it draws a
                   connection's with, weight included. -->
              <h3 v-else class="min-w-0 flex-1 truncate text-sm">{{ row.group.name }}</h3>

              <!-- A pinned group is drawn above the ones it was made beside, and this is the only
                   thing that says why: a list that reorders itself without a word is one people
                   stop trusting. -->
              <Pin
                v-if="row.group.pinned"
                :size="11"
                class="shrink-0 text-primary"
                role="img"
                :aria-label="t('sessions.list.groupPinned')"
              />

              <!-- A group's box is about what is under it: the sessions are what the actions act on. -->
              <button
                type="button"
                data-group-select
                class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground/55 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none"
                :class="
                  groupPicked(row.group) !== 'none' || library.selecting
                    ? 'opacity-100'
                    : 'opacity-0 group-hover:opacity-100'
                "
                :title="
                  groupPicked(row.group) === 'all'
                    ? t('sessions.list.deselectGroup', { name: row.group.name })
                    : t('sessions.list.selectGroup', { name: row.group.name })
                "
                :aria-label="
                  groupPicked(row.group) === 'all'
                    ? t('sessions.list.deselectGroup', { name: row.group.name })
                    : t('sessions.list.selectGroup', { name: row.group.name })
                "
                :aria-checked="groupPicked(row.group) === 'partial' ? 'mixed' : groupPicked(row.group) === 'all'"
                role="checkbox"
                @mousedown.stop
                @click.stop="pickGroup(row.group)"
              >
                <Check v-if="groupPicked(row.group) === 'all'" :size="12" class="text-primary" />
                <Minus v-else-if="groupPicked(row.group) === 'partial'" :size="12" class="text-primary" />
                <Square v-else :size="12" class="stroke-[1.7]" />
              </button>
            </div>

            <!--
              A session: one row, indented by how deep its group is. What the row carries is what
              cannot live in the menu — an attempt in flight with the way to call it off, the mark a
              failure leaves with the reason behind it, and the tick that picks the row out. Editing,
              deleting and the file browser are in the right-click menu, where the rest of the
              actions on a saved session already are.
            -->
            <template v-if="row.kind === 'session'">
              <!--
                The whole row is what is hovered: a card hung off the name alone starts where the
                name ends, which is exactly where the tick and the failure mark sit — it would cover
                the controls of the row it is describing.
              -->
              <HoverCardRoot :open-delay="0" :close-delay="30">
                <HoverCardTrigger as-child>
                  <div
                    :data-row="row.key"
                    class="group relative flex items-center gap-1 rounded-md py-0.5 pr-1 transition-colors hover:bg-accent/60"
                    :class="[
                      markOf(row.key) === 'ring' ? 'bg-primary/10 ring-1 ring-primary/50' : '',
                      flashedRow === row.key ? 'row-located' : '',
                      isSelected(row.session.id) ? 'bg-primary/10' : '',
                    ]"
                    :style="indent(row.depth)"
                    @contextmenu="openSessionMenu($event, row.session)"
                    @mousedown="startDrag($event, { kind: 'session', id: row.session.id }, row.session.name)"
                  >
                    <!-- The row says a name and an address; hovering it says the rest. -->
                    <button
                      type="button"
                      class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1 text-left"
                      :title="t('sessions.list.open', { name: row.session.name })"
                      @click="terminals.open(row.session)"
                    >
                      <!--
                        The mark of the machine, once its host has said which machine it is. Until
                        then — and for a system this build has no mark for — the row keeps the plain
                        terminal it always had: a row is a saved session first and a host second.
                        The mark keeps its own default box, which is the wider one: a picture with
                        white space around it has to be drawn larger to read the same size as the
                        glyph it replaces.
                      -->
                      <SystemIcon
                        v-if="hasMark(row.session)"
                        :system="terminals.systemOf(row.session)"
                        :label="markTitle(row.session)"
                      />
                      <Terminal v-else :size="13" class="shrink-0 text-muted-foreground" />
                      <span class="min-w-0 flex-1">
                        <span class="block truncate text-sm">{{ row.session.name }}</span>
                        <span class="block truncate font-mono text-[10px] text-muted-foreground">{{
                          endpoint(row.session)
                        }}</span>
                      </span>
                    </button>

                    <!-- What did not connect stays on the row it is about: nothing came up, so there is
                     no tab to draw it in, and the row is where the click was made. -->
                    <SessionFailure
                      v-if="failureOf(row.session)"
                      :failure="failureOf(row.session)!"
                      :name="row.session.name"
                      @dismiss="terminals.forgetFailure(row.session.id)"
                    />

                    <!-- An attempt in flight, and the way to call it off, on the row that started it. -->
                    <LoaderCircle
                      v-if="isConnecting(row.session)"
                      :size="12"
                      class="shrink-0 animate-spin text-muted-foreground"
                      :aria-label="t('sessions.list.connecting', { name: row.session.name })"
                    />
                    <button
                      v-if="isConnecting(row.session)"
                      type="button"
                      data-session-cancel
                      class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                      :title="t('sessions.list.cancelConnecting', { name: row.session.name })"
                      :aria-label="t('sessions.list.cancelConnecting', { name: row.session.name })"
                      @mousedown.stop
                      @click.stop="terminals.cancelOpen(row.session.id)"
                    >
                      <X :size="12" />
                    </button>

                    <!-- The tick that picks one out: there when the pointer is, and always there while
                     anything else is picked. -->
                    <button
                      v-if="!isConnecting(row.session)"
                      type="button"
                      data-session-select
                      class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground/55 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none"
                      :class="
                        isSelected(row.session.id) || library.selecting
                          ? 'opacity-100'
                          : 'opacity-0 group-hover:opacity-100'
                      "
                      :title="
                        isSelected(row.session.id)
                          ? t('sessions.list.deselect', { name: row.session.name })
                          : t('sessions.list.select', { name: row.session.name })
                      "
                      :aria-label="
                        isSelected(row.session.id)
                          ? t('sessions.list.deselect', { name: row.session.name })
                          : t('sessions.list.select', { name: row.session.name })
                      "
                      :aria-pressed="isSelected(row.session.id)"
                      @mousedown.stop
                      @click.stop="pickSession($event, row.session)"
                    >
                      <Check v-if="isSelected(row.session.id)" :size="12" class="text-primary" />
                      <Square v-else :size="12" class="stroke-[1.7]" />
                    </button>
                  </div>
                </HoverCardTrigger>
                <SessionDetails :session="row.session" :groups="library.groups" />
              </HoverCardRoot>
            </template>
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

    <!--
      What used to sit here was a list of protocols — one line, saying the thing this application
      obviously does — and a dot reporting that the backend answered. Neither was a control and
      neither answered a question anybody had; a status line is what a page says when it has nothing
      else on it, and a panel that holds a person's servers does.
    -->

    <!--
      Deleting several at once asks the same way one does, and names what is about to go: a count
      alone is a question somebody answers without knowing what they said yes to.
    -->
    <ConfirmDialog
      v-if="deleteSelectedOpen"
      :open="true"
      :title="t('sessions.list.removeSelected', { count: library.selected.size }, library.selected.size)"
      :message="t('sessions.list.removeSelectedMessage', { count: library.selected.size }, library.selected.size)"
      :details="selectedNames"
      :confirm-label="t('sessions.list.removeSelected', { count: library.selected.size }, library.selected.size)"
      danger
      @confirm="confirmDeleteSelected()"
      @update:open="deleteSelectedOpen = false"
    />

    <!-- What the export file will hold, asked once, before a file is asked for. -->
    <ExportSessionsDialog
      :open="exportOpen"
      :busy="exportBusy"
      @confirm="runExport($event)"
      @unencrypted="runExport(null)"
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
    <!--
      The edge, and the drag that moves it. Drawn inside the panel and hanging over its edge so the
      target is easy to hit; the width it reports goes to the settings, because a sidebar somebody
      widened is a sidebar they want wide the next time the application opens.
    -->
    <SidebarResizeHandle
      :width="preferences.resolvedSidebarWidth"
      @move="preferences.setSidebarWidth($event)"
      @settle="preferences.rememberSidebarWidth()"
      @reset="preferences.resetSidebarWidth()"
    />
  </aside>
</template>
