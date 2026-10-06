/**
 * The sidebar's tree handling, as a person meets it: the rows, the prompts, the menus and the drag.
 *
 * The store's suite already says what a write does to the tree. What can only be seen here is the
 * wiring — that the drawer of a heading opens the form that renames *that* group, that the drop a
 * pointer would make is the one the release makes, and that a drop which would do nothing draws no
 * indicator and writes nothing at all.
 *
 * jsdom lays nothing out, so the drag cases give the rows a made-up place on screen and answer
 * `elementFromPoint` from it: the pointer machinery is real (press, travel, release), only the
 * geometry is the test's.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(async () => []),
  sessionSave: vi.fn(async (session: Session) => session),
  sessionDelete: vi.fn(async () => {}),
  sessionsExport: vi.fn(async () => undefined),
  sessionsImport: vi.fn(async () => undefined),
  groupsList: vi.fn(async () => []),
  groupSave: vi.fn(async (group: SessionGroup) => group),
  groupDelete: vi.fn(async () => {}),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(async () => {}),
}));

import AppSidebar from "@/components/shell/AppSidebar.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { useToast } from "@/composables/useToast";
import { i18n } from "@/i18n";
import { groupDelete, groupSave, sessionSave, sessionsExport, sessionsImport } from "@/lib/backend/tauri";
import type { MenuAction, MenuEntry, MenuSubmenu } from "@/lib/menu/types";
import { useSessionsStore } from "@/stores/sessions";
import type { Session, SessionGroup } from "@/types/session";

const work: SessionGroup = { id: "group-1", name: "Work", order: 0 };
const lab: SessionGroup = { id: "group-2", name: "Lab", order: 1 };
const rack: SessionGroup = { id: "group-3", name: "Rack", order: 0, parentId: work.id };

const web: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};
const db: Session = { ...web, id: "session-2", name: "db", groupId: work.id };
/** A session that authenticates a different way, which is what the method filter is about. */
const keyed: Session = { ...web, id: "session-3", name: "keyed", auth: { method: "publicKey", hasSecret: false } };

let host: HTMLDivElement;
let app: App | undefined;

/** Mounts the sidebar over a library the store is simply told about, which is what it draws. */
async function render(groups: readonly SessionGroup[], sessions: readonly Session[]): Promise<void> {
  const library = useSessionsStore();
  library.groups = [...groups];
  library.sessions = [...sessions];

  app = createApp({ render: () => h(AppSidebar) }).use(i18n);
  app.mount(host);
  await nextTick();
}

function heading(text: string): HTMLElement | null {
  return [...host.querySelectorAll("h3")].find((element) => element.textContent?.trim() === text) ?? null;
}

function headingRow(text: string): HTMLElement | null {
  return heading(text)?.parentElement ?? null;
}

function buttonIn(scope: HTMLElement | null, label: string): HTMLElement | null {
  return scope?.querySelector<HTMLElement>(`button[aria-label="${label}"]`) ?? null;
}

function groupNameField(): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>('input[aria-label="Group name"]');
}

/**
 * The confirmation, which is teleported out of the sidebar and onto the body.
 *
 * That is the point of it: a question drawn inside the list would be answered from inside the list's
 * own indentation and scroll position, and two of them could be open at once.
 */
function confirmDialog(): HTMLElement {
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!dialog) {
    throw new Error("no confirmation is on screen");
  }
  return dialog;
}

/** One of the dialog's own buttons, by the words on it. */
function dialogButton(text: string): HTMLElement | undefined {
  return [...confirmDialog().querySelectorAll<HTMLElement>("button")].find(
    (button) => button.textContent?.trim() === text,
  );
}

/** The export's own question: whether the passwords go into the file. */
function exportCheckbox(): HTMLInputElement {
  const box = confirmDialog().querySelector<HTMLInputElement>('input[type="checkbox"]');
  if (!box) {
    throw new Error("the export dialog has no question about passwords");
  }
  return box;
}

function rowElement(key: string): HTMLElement {
  const row = host.querySelector<HTMLElement>(`[data-row="${key}"]`);
  if (!row) {
    throw new Error(`there is no row for ${key}`);
  }
  return row;
}

/** The same lookup without the throw, for asking whether a row is there at all. */
function rowOrNull(key: string): HTMLElement | null {
  return host.querySelector<HTMLElement>(`[data-row="${key}"]`);
}

/** How far a row is indented, which is how far inside the tree it is drawn. */
function indentOf(key: string): number {
  return Number.parseInt(rowElement(key).style.paddingLeft, 10);
}

function searchField(): HTMLInputElement {
  const field = host.querySelector<HTMLInputElement>('input[aria-label="Search sessions"]');
  if (!field) {
    throw new Error("the search field is not drawn");
  }
  return field;
}

function filterButton(): HTMLElement {
  const button = host.querySelector<HTMLElement>("[data-session-filter]");
  if (!button) {
    throw new Error("the filter button is not drawn");
  }
  return button;
}

/** The filter panel's row for one authentication method, by the words on it. */
function methodRow(label: string): HTMLElement | undefined {
  const panel = host.querySelector<HTMLElement>("[data-session-filter-panel]");
  return [...(panel?.querySelectorAll<HTMLElement>("button") ?? [])].find(
    (button) => button.textContent?.trim() === label,
  );
}

function toastMessage(): string {
  return useToast().message.value;
}

/**
 * The rows in the order they are drawn, which is the whole of what the tree looks like.
 *
 * Only rows a drag can start from or land on carry the marker: the line that says a group is empty
 * is text rather than a row, and it is asserted through the text itself.
 */
function drawnRows(): (string | undefined)[] {
  return [...host.querySelectorAll<HTMLElement>("[data-row]")].map((row) => row.dataset.row);
}

async function rightClick(element: HTMLElement | null): Promise<readonly MenuEntry[]> {
  element?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await nextTick();
  return useContextMenu().entries.value;
}

/** The menu an ordinary button opens, which is a click rather than a right-click. */
async function clickMenu(element: HTMLElement | null): Promise<readonly MenuEntry[]> {
  element?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  await nextTick();
  return useContextMenu().entries.value;
}

/** A button by the words on it, for the ones drawn as a sentence rather than as an icon. */
function textButton(scope: HTMLElement | null, text: string): HTMLElement | undefined {
  return [...(scope?.querySelectorAll<HTMLElement>("button") ?? [])].find(
    (button) => button.textContent?.trim() === text,
  );
}

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function submenu(entries: readonly MenuEntry[], id: string): MenuSubmenu {
  const found = entries.find((entry): entry is MenuSubmenu => entry.kind === "submenu" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no submenu called ${id}`);
  }
  return found;
}

async function typeInto(field: HTMLInputElement, value: string): Promise<void> {
  field.value = value;
  field.dispatchEvent(new Event("input"));
  await nextTick();
}

/** Opens the field over a group's name the way the list opens it: through the heading's own menu. */
async function openRename(name: string): Promise<void> {
  const entries = await rightClick(headingRow(name));
  pick(entries, "rename-group").run();
  await nextTick();
}

/** Presses Enter in a field, which is how a rename in place is written. */
async function pressEnter(field: HTMLInputElement): Promise<void> {
  field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await nextTick();
}

/** Where the list itself is, made up the way the rows are: it starts at the top and ends at 300. */
const LIST_BOTTOM = 300;

function listBox(): HTMLElement {
  const list = host.querySelector<HTMLElement>("[data-session-list]");
  if (!list) {
    throw new Error("the list itself is not drawn");
  }
  return list;
}

/**
 * Gives every row a place on screen, and answers "what is at this point" from it.
 *
 * One row per 30 pixels starting at 100, which is enough for the banding to mean something: a group
 * row is 30 tall, so its top third, middle and bottom third are three different answers.
 */
function layOutRows(): void {
  const rows = [...host.querySelectorAll<HTMLElement>("[data-row]")];
  rows.forEach((row, index) => {
    const top = 100 + index * 30;
    row.getBoundingClientRect = () =>
      ({
        top,
        height: 30,
        bottom: top + 30,
        left: 0,
        right: 220,
        width: 220,
        x: 0,
        y: top,
        toJSON: () => ({}),
      }) as DOMRect;
  });
  listBox().getBoundingClientRect = () =>
    ({
      top: 0,
      height: LIST_BOTTOM,
      bottom: LIST_BOTTOM,
      left: 0,
      right: 240,
      width: 240,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
  document.elementFromPoint = (_x: number, y: number): Element | null =>
    rows.find((row) => {
      const box = row.getBoundingClientRect();
      return y >= box.top && y < box.bottom;
    }) ?? null;
}

type Band = "top" | "middle" | "bottom";

function bandY(key: string, band: Band): number {
  const box = rowElement(key).getBoundingClientRect();
  if (band === "top") {
    return box.top + 3;
  }
  if (band === "bottom") {
    return box.bottom - 3;
  }
  return box.top + 15;
}

/** Presses a row and travels far enough for the press to become a drag. */
async function startDrag(key: string): Promise<void> {
  layOutRows();
  const box = rowElement(key).getBoundingClientRect();
  rowElement(key).dispatchEvent(
    new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 50, clientY: box.top + 15 }),
  );
  window.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 50, clientY: box.top + 27 }));
  await nextTick();
}

/** Moves the pointer over a row — into a band of it — without letting go. */
async function dragOver(key: string, band: Band = "middle"): Promise<void> {
  window.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 50, clientY: bandY(key, band) }));
  await nextTick();
}

async function drop(): Promise<void> {
  window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, clientX: 50, clientY: 400 }));
  await nextTick();
}

async function drag(from: string, to: string, band: Band = "middle"): Promise<void> {
  await startDrag(from);
  await dragOver(to, band);
  await drop();
}

function ghost(): HTMLElement | null {
  return host.querySelector<HTMLElement>("[data-drag-ghost]");
}

/** Whether a row is drawn as the place a drop would land. */
function marked(key: string): boolean {
  const classes = rowElement(key).className;
  return classes.includes("ring-primary") || classes.includes("bg-primary/10");
}

/**
 * Puts every backend mock back to the answer a working backend gives.
 *
 * `mockClear` only forgets the calls: a test that asked for a refusal — and one does, to prove a
 * group is not made when its write is refused — would otherwise leave the *next* test with a
 * backend that refuses everything, and it would fail for a reason that has nothing to do with it.
 */
function resetBackend(): void {
  vi.mocked(groupSave)
    .mockReset()
    .mockImplementation(async (group: SessionGroup) => group);
  vi.mocked(groupDelete)
    .mockReset()
    .mockImplementation(async () => {});
  vi.mocked(sessionSave)
    .mockReset()
    .mockImplementation(async (session: Session) => session);
  vi.mocked(sessionsExport).mockReset().mockResolvedValue(undefined);
  vi.mocked(sessionsImport).mockReset().mockResolvedValue(undefined);
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  resetBackend();
  // jsdom runs no animation frames unless it was asked to; the drag scrolls on them.
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 1));
  vi.stubGlobal("cancelAnimationFrame", (handle: number) => window.clearTimeout(handle));
});

afterEach(() => {
  app?.unmount();
  host.remove();
  // A drag that ended in a drop leaves one listener that eats the next click; that is what keeps
  // letting go of a row from opening it. In the window, the next click is the person's own — but
  // between tests there is nobody to click, and the listener would be left to eat the first click
  // of whatever runs next. This is that click.
  window.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  window.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  vi.unstubAllGlobals();
});

describe("the rows", () => {
  it("draws a group before what is in it, and says when a group is empty", async () => {
    await render([work, lab], [web, db]);

    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "group:group-2", "session:session-1"]);
    expect(host.textContent).toContain("Nothing in this group.");
  });

  it("draws a session that is in no group at the root, with no heading over it", async () => {
    await render([work], [web, db]);

    // The heading that used to sit here said what the row already says. Nothing replaces it: the
    // session is drawn at the same indent as a group, at the end of the list.
    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "session:session-1"]);
    expect(indentOf("session:session-1")).toBe(indentOf("group:group-1"));
    expect([...host.querySelectorAll("h3")].map((heading) => heading.textContent?.trim())).toEqual(["Work"]);
  });

  it("indents a nested group further than the group it sits in", async () => {
    await render([work, rack], [db]);

    const outer = Number.parseInt(rowElement("group:group-1").style.paddingLeft, 10);
    const inner = Number.parseInt(rowElement("group:group-3").style.paddingLeft, 10);

    expect(inner).toBeGreaterThan(outer);
  });

  it("closes a group with the chevron, and opens it again", async () => {
    await render([work], [web, db]);

    buttonIn(headingRow("Work"), "Collapse Work")?.click();
    await nextTick();

    expect(drawnRows()).toEqual(["group:group-1", "session:session-1"]);

    buttonIn(headingRow("Work"), "Expand Work")?.click();
    await nextTick();

    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "session:session-1"]);
  });

  it("draws no buttons on a group heading: changing one is a right-click away", async () => {
    await render([work], [web]);

    // The chevron is the only button on the row. Renaming and deleting live in the menu, so the
    // heading is a name with a folder beside it rather than a strip of icons.
    const row = headingRow("Work");
    const labels = [...(row?.querySelectorAll("button") ?? [])].map((button) => button.getAttribute("aria-label"));
    expect(labels).toEqual(["Collapse Work"]);

    const entries = await rightClick(row);
    const actions = entries.filter((entry) => entry.kind === "action").map((entry) => entry.id);
    expect(actions).toContain("rename-group");
    expect(actions).toContain("remove-group");
  });

  it("draws the folder a group stands for, and the open one when it is open", async () => {
    await render([work], [db]);

    // A folder is what the row is; the two shapes are the two states, the way the file browser
    // draws its own.
    expect(headingRow("Work")?.querySelector("svg.lucide-folder-open")).not.toBeNull();

    buttonIn(headingRow("Work"), "Collapse Work")?.click();
    await nextTick();

    expect(headingRow("Work")?.querySelector("svg.lucide-folder-open")).toBeNull();
    expect(headingRow("Work")?.querySelector("svg.lucide-folder")).not.toBeNull();
  });

  it("draws no row for leaving every group until something is being dragged", async () => {
    await render([work], [db]);

    // What that row is for exists only while a drag is: the rest of the time the list is the
    // sessions and the folders, and nothing that is only a place to put something.
    expect(rowOrNull("drop-out")).toBeNull();

    await startDrag("session:session-2");
    layOutRows();

    expect(rowOrNull("drop-out")).not.toBeNull();

    await drop();
    expect(rowOrNull("drop-out")).toBeNull();
  });
});

describe("making a group", () => {
  it("makes one in a single click, and puts its name in the row to type over", async () => {
    await render([], [web]);

    buttonIn(host, "New group")?.click();

    // The row that appears is a group with its name in a field: what the person sees is the group
    // they asked for, already there, with the name ready to be typed over.
    await vi.waitFor(() => expect(groupNameField()).not.toBeNull());
    expect(groupNameField()?.value).toBe("New group");

    // One write, with the name it is drawn under until somebody says otherwise.
    const written = vi.mocked(groupSave).mock.calls[0]?.[0];
    expect(written).toMatchObject({ name: "New group", order: 0 });
    expect(written?.parentId).toBeUndefined();
    expect(written?.id).toBeTruthy();
    expect(drawnRows()[0]?.startsWith("group:"), "and it is drawn where it landed").toBe(true);
  });

  it("writes the name that was typed over it", async () => {
    await render([], [web]);
    buttonIn(host, "New group")?.click();
    await vi.waitFor(() => expect(groupNameField()).not.toBeNull());
    vi.mocked(groupSave).mockClear();

    const field = groupNameField();
    if (field) {
      await typeInto(field, "Lab");
      await pressEnter(field);
    }

    await vi.waitFor(() => expect(vi.mocked(groupSave)).toHaveBeenCalled());
    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ name: "Lab" });
    await vi.waitFor(() => expect(heading("Lab")).not.toBeNull());
    expect(groupNameField(), "and the field closes once it worked").toBeNull();
  });

  it("leaves the group named as it was made when the name is emptied", async () => {
    // Nothing is thrown away: the group exists, and an empty name is somebody changing their mind
    // about the name rather than about the group.
    await render([], [web]);
    buttonIn(host, "New group")?.click();
    await vi.waitFor(() => expect(groupNameField()).not.toBeNull());
    vi.mocked(groupSave).mockClear();

    const field = groupNameField();
    if (field) {
      await typeInto(field, "   ");
      await pressEnter(field);
    }
    await nextTick();

    expect(vi.mocked(groupSave)).not.toHaveBeenCalled();
    expect(heading("New group"), "the group is still there, under the name it was made with").not.toBeNull();
  });

  it("lets a narrowed list out of its filter, so the group it made can be named", async () => {
    // The name is typed in the row, and a row the search is hiding cannot be typed into: a group
    // that is asked for and does not appear is worse than a search that is cleared by asking.
    await render([], [web]);
    await typeInto(searchField(), "nothing-like-this-at-all");
    expect(drawnRows()).toEqual([]);

    buttonIn(host, "New group")?.click();

    await vi.waitFor(() => expect(groupNameField()).not.toBeNull());
    expect(searchField().value).toBe("");
    expect(drawnRows()[0]?.startsWith("group:")).toBe(true);
  });

  it("makes one inside a group through that group's own menu, and opens it so it can be seen", async () => {
    await render([work, rack], [db]);
    useSessionsStore().setCollapsed(work.id, true);
    await nextTick();
    expect(heading("Rack")).toBeNull();

    const entries = await rightClick(headingRow("Work"));
    pick(entries, "add-group-inside").run();

    await vi.waitFor(() => expect(groupNameField()).not.toBeNull());
    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ name: "New group", parentId: "group-1" });
    // The group it was made in opens: a group that was just made and cannot be seen is not made.
    // Its own name is in a field, so what says it is there is the field's place in the list.
    const keys = drawnRows();
    expect(keys.slice(0, 2), "Work opens and Rack is drawn again").toEqual(["group:group-1", "group:group-3"]);
    expect(keys[2]?.startsWith("group:"), "the group that was just made is drawn inside Work").toBe(true);
    expect(keys[3]).toBe("session:session-2");
  });
});

describe("a group made for a session", () => {
  it("makes the group and moves the session into it, in that order", async () => {
    await render([], [web]);
    vi.mocked(groupSave).mockImplementation(async (group) => group);

    const entries = await rightClick(host.querySelector<HTMLElement>('[title="Open a terminal for web"]'));
    pick(entries, "move-to-new-group").run();

    await vi.waitFor(() => expect(vi.mocked(sessionSave)).toHaveBeenCalled());
    const made = vi.mocked(groupSave).mock.calls[0]?.[0];
    const moved = vi.mocked(sessionSave).mock.calls[0]?.[0];
    expect(moved).toMatchObject({ id: "session-1", groupId: made?.id });
    expect(made?.name).toBe("New group");
  });

  it("offers it inside the list of groups when there are any", async () => {
    await render([work], [db]);

    const entries = await rightClick(host.querySelector<HTMLElement>('[title="Open a terminal for db"]'));
    const move = submenu(entries, "move-to-group");
    const ids = move.entries.map((entry) => (entry.kind === "action" ? entry.id : entry.kind));

    expect(ids).toEqual(["move-to-ungrouped", "move-to-group-1", "separator", "move-to-new-group"]);
  });

  it("makes no group when the write is refused, and moves nothing", async () => {
    await render([], [web]);
    vi.mocked(groupSave).mockRejectedValue({
      version: 1,
      code: "STORE-IO",
      messageKey: "errors.store.io",
      messageParams: {},
      origin: { subsystem: "store" },
      outcome: "notSent",
    });

    const entries = await rightClick(host.querySelector<HTMLElement>('[title="Open a terminal for web"]'));
    pick(entries, "move-to-new-group").run();

    await vi.waitFor(() => expect(useSessionsStore().failure?.code).toBe("STORE-IO"));
    expect(vi.mocked(sessionSave)).not.toHaveBeenCalled();
  });
});

describe("changing a group", () => {
  it("renames where the name is written, through the heading's menu", async () => {
    await render([work, lab], [web, db]);

    await openRename("Work");
    const field = groupNameField();
    expect(field?.value, "the field starts from the name it is changing").toBe("Work");
    if (field) {
      await typeInto(field, "Work (EU)");
      await pressEnter(field);
    }
    await vi.waitFor(() => expect(vi.mocked(groupSave)).toHaveBeenCalled());

    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ id: "group-1", name: "Work (EU)" });
    await vi.waitFor(() => expect(heading("Work (EU)")).not.toBeNull());
    // The field goes and the label comes back, in the row the name belongs to.
    expect(groupNameField()).toBeNull();
  });

  it("writes nothing when the name did not change", async () => {
    await render([work], [db]);

    await openRename("Work");
    const field = groupNameField();
    if (field) {
      await pressEnter(field);
    }

    expect(vi.mocked(groupSave)).not.toHaveBeenCalled();
    expect(groupNameField()).toBeNull();
    expect(heading("Work")).not.toBeNull();
  });

  it("writes nothing, and does not delete anything, when the name is emptied", async () => {
    await render([work], [db]);

    await openRename("Work");
    const field = groupNameField();
    if (field) {
      await typeInto(field, "   ");
      await pressEnter(field);
    }

    expect(vi.mocked(groupSave)).not.toHaveBeenCalled();
    expect(vi.mocked(groupDelete)).not.toHaveBeenCalled();
    expect(heading("Work")).not.toBeNull();
  });

  it("cancels on Escape, putting the old name back", async () => {
    await render([work], [db]);

    await openRename("Work");
    const field = groupNameField();
    if (field) {
      await typeInto(field, "Nonsense");
      field.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    }
    await nextTick();

    expect(vi.mocked(groupSave), "Escape is not a write, and the blur it causes is not either").not.toHaveBeenCalled();
    expect(useSessionsStore().groups[0]?.name).toBe("Work");
    expect(groupNameField()).toBeNull();
  });

  it("writes the name when the field is left, which is how a rename in place usually ends", async () => {
    await render([work], [db]);

    await openRename("Work");
    const field = groupNameField();
    if (field) {
      await typeInto(field, "Desk");
      field.dispatchEvent(new Event("blur"));
    }

    await vi.waitFor(() => expect(vi.mocked(groupSave)).toHaveBeenCalled());
    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ id: "group-1", name: "Desk" });
  });

  it("asks before deleting, says what happens to what is inside, and keeps the sessions", async () => {
    await render([work], [db]);

    const entries = await rightClick(headingRow("Work"));
    pick(entries, "remove-group").run();
    await nextTick();

    // The question is a dialog of its own, over the list rather than in the heading — nothing has
    // happened yet, and the row the question is about is still drawn where it was.
    expect(confirmDialog().textContent).toContain("Delete the group “Work”?");
    expect(confirmDialog().textContent).toContain("move out of every group");
    expect(heading("Work")).not.toBeNull();

    dialogButton("Delete group")?.click();
    await vi.waitFor(() => expect(vi.mocked(groupDelete)).toHaveBeenCalledWith("group-1"));

    await vi.waitFor(() => expect(heading("Work")).toBeNull());
    // The session that was in it is still on screen, now at the root.
    expect(drawnRows()).toEqual(["session:session-2"]);
    expect(useSessionsStore().sessions.map((session) => session.id)).toEqual(["session-2"]);
  });

  it("lets the confirmation be called off", async () => {
    await render([work], [db]);

    const entries = await rightClick(headingRow("Work"));
    pick(entries, "remove-group").run();
    await nextTick();
    dialogButton("Cancel")?.click();
    await nextTick();
    await nextTick();

    expect(vi.mocked(groupDelete)).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(heading("Work")).not.toBeNull();
  });
});

describe("a group's menu", () => {
  it("opens on the heading with the group's actions, and the ends are drawn but not movable", async () => {
    await render([work, lab], []);

    const entries = await rightClick(headingRow("Work"));

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "rename-group",
      "add-group-inside",
      "move-group-up",
      "move-group-down",
      "remove-group",
    ]);
    expect(pick(entries, "move-group-up").disabled, "the first group has nothing above it").toBe(true);
    expect(pick(entries, "move-group-down").disabled).toBe(false);
  });

  it("moves the group a place through the menu", async () => {
    await render([work, lab], []);

    const entries = await rightClick(headingRow("Lab"));
    pick(entries, "move-group-up").run();

    await vi.waitFor(() => expect(vi.mocked(groupSave)).toHaveBeenCalledTimes(2));
    expect(vi.mocked(groupSave).mock.calls.map(([group]) => [group.id, group.order])).toEqual([
      ["group-2", 0],
      ["group-1", 1],
    ]);
  });
});

describe("a session's menu", () => {
  it("moves the session through the list of groups, which is the way in without a drag", async () => {
    await render([work, lab], [web, db]);

    const entries = await rightClick(host.querySelector<HTMLElement>('[title="Open a terminal for web"]'));
    const move = submenu(entries, "move-to-group");
    expect(pick(move.entries, "move-to-ungrouped").checked).toBe(true);

    pick(move.entries, "move-to-group-2").run();
    await vi.waitFor(() => expect(vi.mocked(sessionSave)).toHaveBeenCalled());

    expect(vi.mocked(sessionSave).mock.calls[0]?.[0]).toMatchObject({ id: "session-1", groupId: "group-2" });
    await vi.waitFor(() =>
      expect(drawnRows().indexOf("session:session-1")).toBeGreaterThan(drawnRows().indexOf("group:group-2")),
    );
  });
});

describe("dragging", () => {
  it("moves a session into the group it was dropped on", async () => {
    await render([work, lab], [web, db]);
    const atRoot = indentOf("session:session-1");

    await startDrag("session:session-1");
    expect(ghost()?.textContent?.trim(), "what is moving is drawn under the pointer").toBe("web");

    await dragOver("group:group-2");
    expect(marked("group:group-2"), "the group it would join is drawn as the place").toBe(true);
    await drop();

    expect(vi.mocked(sessionSave)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(sessionSave).mock.calls[0]?.[0]).toMatchObject({ id: "session-1", groupId: "group-2" });
    await vi.waitFor(() => expect(drawnRows()).toContain("session:session-1"));
    expect(drawnRows().indexOf("session:session-1")).toBeGreaterThan(drawnRows().indexOf("group:group-2"));
    // Drawn inside the group, which is what the indent means: the row is one step further in than
    // it was when it sat at the root.
    expect(indentOf("session:session-1")).toBeGreaterThan(atRoot);
  });

  it("takes a session out of every group when it is dropped on the row drawn for it", async () => {
    await render([work], [web, db]);
    const inside = indentOf("session:session-2");

    await startDrag("session:session-2");
    // The row appears with the drag, so it has to be given a place on screen before the pointer can
    // be over it — jsdom lays nothing out.
    layOutRows();
    await dragOver("drop-out");
    expect(marked("drop-out"), "the row says it is where the session would go").toBe(true);
    await drop();

    expect(vi.mocked(sessionSave).mock.calls[0]?.[0]).toMatchObject({ id: "session-2", groupId: undefined });
    await vi.waitFor(() => expect(indentOf("session:session-2")).toBeLessThan(inside));
  });

  it("puts a group inside the one it was dropped on, and beside another at the edges", async () => {
    await render([work, lab], []);

    await drag("group:group-2", "group:group-1", "middle");

    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ id: "group-2", parentId: "group-1", order: 0 });
  });

  it("puts a group beside another one when the edge of the row was the target", async () => {
    await render([work, lab], []);

    await drag("group:group-2", "group:group-1", "top");

    expect(vi.mocked(groupSave).mock.calls[0]?.[0]).toMatchObject({ id: "group-2", parentId: undefined, order: 0 });
  });

  it("refuses to put a group under something that is already under it, and draws no place for it", async () => {
    await render([work, rack], []);

    await startDrag("group:group-1");
    await dragOver("group:group-3");
    expect(marked("group:group-3"), "a drop that would be refused is no target at all").toBe(false);
    await drop();

    expect(vi.mocked(groupSave)).not.toHaveBeenCalled();
  });

  it("writes nothing when a session is dropped on the group it is already in", async () => {
    await render([work, lab], [web, db]);

    await startDrag("session:session-2");
    await dragOver("group:group-1");
    expect(marked("group:group-1")).toBe(false);
    await drop();

    expect(vi.mocked(sessionSave)).not.toHaveBeenCalled();
  });

  it("lets go without moving anything when the drag is abandoned", async () => {
    await render([work, lab], [web]);

    await startDrag("session:session-1");
    await dragOver("group:group-2");
    expect(marked("group:group-2")).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    await drop();

    expect(vi.mocked(sessionSave)).not.toHaveBeenCalled();
    expect(ghost()).toBeNull();
    expect(marked("group:group-2")).toBe(false);
  });

  it("leaves a plain press alone: a click that never travelled is still a click", async () => {
    await render([work], [db]);

    const row = rowElement("session:session-2");
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 50, clientY: 100 }));
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, clientX: 50, clientY: 100 }));
    await nextTick();

    expect(vi.mocked(sessionSave)).not.toHaveBeenCalled();
    expect(ghost()).toBeNull();
  });

  it("swallows the click that follows a drag, so letting go over a row does not open it", async () => {
    await render([work, lab], [web, db]);

    await drag("session:session-1", "group:group-2");

    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    rowElement("session:session-1").dispatchEvent(click);
    expect(click.defaultPrevented, "the click the press would have become is stopped").toBe(true);

    // And it is only that one: the next press takes the guard away.
    const later = new MouseEvent("click", { bubbles: true, cancelable: true });
    rowElement("session:session-1").dispatchEvent(later);
    expect(later.defaultPrevented).toBe(false);
  });

  it("scrolls the list while a drag is held against its end, so a group out of sight can be reached", async () => {
    await render([work, lab], [web, db]);
    const list = listBox();
    let scrolled = 0;
    Object.defineProperty(list, "scrollTop", {
      get: () => scrolled,
      set: (value: number) => {
        scrolled = value;
      },
      configurable: true,
    });

    await startDrag("session:session-1");
    window.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 50, clientY: LIST_BOTTOM - 4 }));
    await nextTick();
    await new Promise((resolve) => window.setTimeout(resolve, 10));

    expect(scrolled).toBeGreaterThan(0);
    await drop();
  });

  it("does not scroll while the pointer is in the middle of the list", async () => {
    await render([work, lab], [web, db]);
    const list = listBox();
    let scrolled = 0;
    Object.defineProperty(list, "scrollTop", {
      get: () => scrolled,
      set: (value: number) => {
        scrolled = value;
      },
      configurable: true,
    });

    await startDrag("session:session-1");
    await dragOver("group:group-2");
    await new Promise((resolve) => window.setTimeout(resolve, 10));

    expect(scrolled).toBe(0);
    await drop();
  });
});

describe("searching the list", () => {
  it("narrows the list to what matches, and keeps the folder a match is in", async () => {
    await render([work, lab], [web, db]);

    await typeInto(searchField(), "db");

    expect(drawnRows()).toEqual(["group:group-1", "session:session-2"]);
  });

  it("opens a folder a match is in, even when it was closed", async () => {
    await render([work], [web, db]);
    buttonIn(headingRow("Work"), "Collapse Work")?.click();
    await nextTick();
    expect(drawnRows()).toEqual(["group:group-1", "session:session-1"]);

    await typeInto(searchField(), "db");

    // The match is why the search ran, so the folder it is in cannot stay shut over it.
    expect(drawnRows()).toEqual(["group:group-1", "session:session-2"]);
  });

  it("says nothing was found, and offers the way back", async () => {
    await render([work], [web, db]);

    await typeInto(searchField(), "nothing-like-this");

    expect(drawnRows()).toEqual([]);
    expect(host.textContent).toContain("No session matches");

    textButton(host, "Show every session")?.click();
    await nextTick();

    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "session:session-1"]);
    expect(searchField().value).toBe("");
  });

  it("empties the field from the button inside it, which is where the search is", async () => {
    await render([work], [web, db]);
    await typeInto(searchField(), "db");
    expect(drawnRows()).toEqual(["group:group-1", "session:session-2"]);

    buttonIn(host, "Clear the search")?.click();
    await nextTick();

    expect(searchField().value).toBe("");
    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "session:session-1"]);
  });

  it("takes the drag away while the list is narrowed", async () => {
    // A drop lands in the list as it is drawn, and while a search is narrowing it the row under the
    // pointer is not the row the store would move something next to. Nothing is offered rather than
    // something guessed at.
    await render([work], [web, db]);
    await typeInto(searchField(), "web");

    await startDrag("session:session-1");

    expect(ghost()).toBeNull();
  });
});

describe("filtering by how a session authenticates", () => {
  it("keeps only the sessions that authenticate the way that was chosen", async () => {
    await render([work], [web, db, keyed]);

    filterButton().click();
    await nextTick();
    methodRow("Public key")?.click();
    await nextTick();

    expect(drawnRows()).toEqual(["session:session-3"]);
    expect(filterButton().getAttribute("aria-expanded")).toBe("true");
  });

  it("closes the panel when the pointer goes somewhere else, and keeps the filter", async () => {
    await render([work], [web, db, keyed]);
    filterButton().click();
    await nextTick();
    methodRow("Public key")?.click();
    await nextTick();

    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    await nextTick();

    expect(host.querySelector("[data-session-filter-panel]")).toBeNull();
    expect(drawnRows()).toEqual(["session:session-3"]);
  });

  it("puts everything back when the filter is taken off", async () => {
    await render([work], [web, db, keyed]);
    filterButton().click();
    await nextTick();
    methodRow("Public key")?.click();
    await nextTick();

    methodRow("Show every session")?.click();
    await nextTick();

    expect(drawnRows()).toEqual(["group:group-1", "session:session-2", "session:session-1", "session:session-3"]);
  });
});

describe("importing and exporting the library", () => {
  it("offers both from the library menu, and has nothing to export from an empty library", async () => {
    await render([], []);

    const entries = await clickMenu(buttonIn(host, "More actions"));

    expect(entries.map((entry) => (entry.kind === "action" ? entry.id : "·"))).toEqual([
      "import-sessions",
      "export-sessions",
    ]);
    expect(pick(entries, "export-sessions").disabled).toBe(true);
  });

  it("asks about the passwords before exporting, and writes with the answer that was given", async () => {
    vi.mocked(sessionsExport).mockResolvedValue({
      sessions: 2,
      groups: 1,
      secrets: 1,
      fileName: "7shell-sessions.json",
    });
    await render([work], [web, db]);

    const entries = await clickMenu(buttonIn(host, "More actions"));
    pick(entries, "export-sessions").run();
    await nextTick();

    // The question starts at the answer that gives nothing away, every time it is asked.
    const ask = exportCheckbox();
    expect(ask.checked).toBe(false);

    ask.checked = true;
    ask.dispatchEvent(new Event("change"));
    await nextTick();
    dialogButton("Export sessions")?.click();

    await vi.waitFor(() => expect(vi.mocked(sessionsExport)).toHaveBeenCalled());
    expect(vi.mocked(sessionsExport).mock.calls[0]).toEqual(["Export sessions", "7shell-sessions.json", true]);
    await vi.waitFor(() => expect(toastMessage()).toContain("2 sessions written to 7shell-sessions.json"));
  });

  it("leaves the passwords behind when the question is left as it was asked", async () => {
    vi.mocked(sessionsExport).mockResolvedValue({
      sessions: 2,
      groups: 1,
      secrets: 0,
      fileName: "7shell-sessions.json",
    });
    await render([work], [web, db]);

    const entries = await clickMenu(buttonIn(host, "More actions"));
    pick(entries, "export-sessions").run();
    await nextTick();
    dialogButton("Export sessions")?.click();

    await vi.waitFor(() => expect(vi.mocked(sessionsExport)).toHaveBeenCalled());
    expect(vi.mocked(sessionsExport).mock.calls[0]?.[2]).toBe(false);
  });

  it("imports what a file holds and says what came with it, warnings and all", async () => {
    vi.mocked(sessionsImport).mockResolvedValue({
      sessions: 3,
      groups: 2,
      secrets: 1,
      missingKeys: 1,
      droppedHops: 0,
    });
    await render([], []);

    const entries = await clickMenu(buttonIn(host, "More actions"));
    pick(entries, "import-sessions").run();

    await vi.waitFor(() => expect(vi.mocked(sessionsImport)).toHaveBeenCalledWith("Import sessions"));
    await vi.waitFor(() =>
      expect(toastMessage()).toBe(
        "Imported 3 sessions, 2 groups, 1 saved password, 1 session names a key this device does not hold.",
      ),
    );
  });

  it("says nothing when the file dialog is dismissed", async () => {
    vi.mocked(sessionsImport).mockResolvedValue(undefined);
    await render([work], [db]);
    const before = toastMessage();

    const entries = await clickMenu(buttonIn(host, "More actions"));
    pick(entries, "import-sessions").run();
    await vi.waitFor(() => expect(vi.mocked(sessionsImport)).toHaveBeenCalled());

    expect(toastMessage()).toBe(before);
    expect(useSessionsStore().failure).toBeNull();
  });

  it("shows a file that cannot be read in the list's own box rather than in a toast", async () => {
    vi.mocked(sessionsImport).mockRejectedValue({
      version: 1,
      code: "SESSION-BUNDLE-INVALID",
      messageKey: "errors.session.bundleInvalid",
      messageParams: {},
      origin: { subsystem: "store" },
      outcome: "notSent",
    });
    await render([work], [db]);

    const entries = await clickMenu(buttonIn(host, "More actions"));
    pick(entries, "import-sessions").run();

    await vi.waitFor(() => expect(host.textContent).toContain("is not a session export"));
    expect(host.textContent).toContain("SESSION-BUNDLE-INVALID");
  });
});
