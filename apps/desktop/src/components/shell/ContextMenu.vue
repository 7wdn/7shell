<script setup lang="ts">
/**
 * Where the context menu is drawn: one floating layer, owned by the shell.
 *
 * What lives here is the menu's *shape* and nothing about its contents — which entries there are, and
 * what running one does, was decided by the region that was right-clicked. Everything that is the
 * same for every menu is here: where it lands, how the keyboard moves inside it, and what closes it.
 *
 * Four of those are worth stating.
 *
 * 1. **It is placed after it is drawn.** Its size is only known once it is in the document, and a
 *    menu positioned from a guess would jump on its first frame.
 * 2. **Two levels, and the second one belongs to a row.** A submenu is drawn against the row that
 *    opened it rather than at the pointer: it is that row's list, and the row stays visible while it
 *    is chosen from. `ArrowLeft` and `Escape` take it away before they take the menu away.
 * 3. **The keyboard is handed back.** Somebody who right-clicked a terminal to paste wants the
 *    terminal back afterwards, not the body of the window. Focus is returned synchronously, before
 *    an action of theirs can open a dialog of its own and claim it.
 * 4. **The active row is announced rather than focused.** Focus stays on a list and
 *    `aria-activedescendant` names the row the keyboard is on, so one element per level keeps
 *    deciding what the arrow keys mean.
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ContextMenuRows from "@/components/shell/ContextMenuRows.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { firstEnabledIndex, lastEnabledIndex, nextEnabledIndex } from "@/lib/menu/navigate";
import { placeMenu, placeSubmenu } from "@/lib/menu/place";
import { type MenuAction, type MenuEntry, type MenuSubmenu, canFocus } from "@/lib/menu/types";

/** What each level's row ids start with. The layer above recomputes the same string for its own use. */
const ROOT = "context-menu-root";
const SUB = "context-menu-sub";

const { t } = useI18n();
const { visible, at, title, entries, closeMenu } = useContextMenu();

const box = ref<HTMLElement | null>(null);
const submenuBox = ref<HTMLElement | null>(null);
const position = ref({ left: 0, top: 0 });
const submenuPosition = ref({ left: 0, top: 0 });
/** Which row the keyboard is on in each level. -1 while nothing is chosen yet. */
const active = ref(-1);
const subActive = ref(-1);
/** The submenu that is open, and which row opened it. */
const open = ref<{ index: number; entry: MenuSubmenu } | null>(null);
/** Where the keyboard was before the menu took it, so it can be given back on close. */
let restoreTo: HTMLElement | null = null;

const label = computed(() => t(title.value.key, title.value.params ?? {}));
// A menu is chrome rather than a document: a drag across it is somebody leaving in a hurry, and a
// selection smeared over a list of verbs is noise. `select-none` on the box is inherited by both
// levels — the rows below are its children. See `docs/ui-conventions.md`.
const boxClass =
  "fixed z-[70] max-w-72 min-w-44 rounded-lg border border-border bg-card py-1 shadow-xl outline-none cursor-default select-none";
const rootActiveId = computed(() => rowId(ROOT, entries.value[active.value]));
const submenuActiveId = computed(() => rowId(SUB, open.value?.entry.entries[subActive.value]));

function rowId(prefix: string, entry: MenuEntry | undefined): string | undefined {
  return entry === undefined ? undefined : `${prefix}-${entry.id}`;
}

/**
 * Closing, and giving the keyboard back, as one act.
 *
 * Every close in here goes through this rather than through `closeMenu`, so that "the menu is gone"
 * and "the person can type again" cannot come apart.
 */
function dismiss(): void {
  open.value = null;
  closeMenu();
  const target = restoreTo;
  restoreTo = null;
  target?.focus();
}

/** Taking the submenu away leaves the menu itself up, with the keyboard back on the row that opened it. */
function closeSubmenu(): void {
  open.value = null;
  void nextTick(() => box.value?.focus());
}

/**
 * The row a submenu is drawn against, measured now: it is placed after it is drawn, for the same
 * reason the menu is — its height is only known once it is in the document.
 */
async function openSubmenu(index: number, entry: MenuSubmenu): Promise<void> {
  open.value = { index, entry };
  subActive.value = -1;
  await nextTick();
  const element = submenuBox.value;
  const row = document.getElementById(`${ROOT}-${entry.id}`);
  if (!element || !row) {
    return;
  }
  const rect = row.getBoundingClientRect();
  submenuPosition.value = placeSubmenu(
    { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
    { width: element.offsetWidth, height: element.offsetHeight },
    { width: window.innerWidth, height: window.innerHeight },
  );
  element.focus();
}

function run(entry: MenuAction): void {
  dismiss();
  void entry.run();
}

/** Pointing at a row moves the keyboard there; pointing at a parent row brings its list out. */
function hover(index: number, entry: MenuEntry, level: "root" | "sub"): void {
  if (!canFocus(entry)) {
    return;
  }
  if (level === "sub") {
    subActive.value = index;
    return;
  }

  active.value = index;
  if (entry.kind === "submenu") {
    if (open.value?.index !== index) {
      void openSubmenu(index, entry);
    }
    return;
  }
  // A row that opens nothing takes the submenu away: the list belongs to the row it came from.
  if (open.value) {
    closeSubmenu();
  }
}

/** Pressing a row runs it — unless it is a parent, which is a switch for the list beside the menu. */
function activate(index: number, entry: MenuEntry, level: "root" | "sub"): void {
  if (entry.kind === "submenu") {
    if (level === "root" && open.value?.index === index) {
      closeSubmenu();
      return;
    }
    void openSubmenu(index, entry);
    return;
  }
  if (entry.kind === "action" && entry.disabled !== true) {
    run(entry);
  }
}

/** Which list a key press is about: the one the keyboard is in. */
function levelOf(event: KeyboardEvent): "root" | "sub" {
  return submenuBox.value !== null && event.currentTarget === submenuBox.value ? "sub" : "root";
}

function levelEntries(level: "root" | "sub"): readonly MenuEntry[] {
  return level === "sub" ? (open.value?.entry.entries ?? []) : entries.value;
}

function levelActive(level: "root" | "sub"): number {
  return level === "sub" ? subActive.value : active.value;
}

function setActive(level: "root" | "sub", index: number): void {
  if (level === "sub") {
    subActive.value = index;
  } else {
    active.value = index;
  }
}

function move(level: "root" | "sub", step: 1 | -1): void {
  const next = nextEnabledIndex(levelEntries(level), levelActive(level), step);
  if (next !== null) {
    setActive(level, next);
  }
}

function onKeydown(event: KeyboardEvent): void {
  const level = levelOf(event);

  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      move(level, 1);
      return;
    case "ArrowUp":
      event.preventDefault();
      move(level, -1);
      return;
    case "Home":
      event.preventDefault();
      setActive(level, firstEnabledIndex(levelEntries(level)) ?? -1);
      return;
    case "End":
      event.preventDefault();
      setActive(level, lastEnabledIndex(levelEntries(level)) ?? -1);
      return;
    case "ArrowRight": {
      const entry = entries.value[active.value];
      if (level === "root" && entry?.kind === "submenu") {
        event.preventDefault();
        void openSubmenu(active.value, entry);
      }
      return;
    }
    case "ArrowLeft":
      if (level === "sub") {
        event.preventDefault();
        closeSubmenu();
      }
      return;
    case "Enter":
    case " ": {
      const entry = levelEntries(level)[levelActive(level)];
      if (entry === undefined) {
        return;
      }
      event.preventDefault();
      activate(levelActive(level), entry, level);
      return;
    }
    case "Escape":
      event.preventDefault();
      // The list beside the menu goes first: it is the innermost thing that is open.
      if (open.value) {
        closeSubmenu();
        return;
      }
      dismiss();
      return;
    case "Tab":
      // Dismissed rather than swallowed: the keyboard goes back to where it came from, which is where
      // this Tab would have carried on from anyway.
      dismiss();
      return;
    default:
      return;
  }
}

function onPointerDown(event: PointerEvent): void {
  const target = event.target as Node;
  if (box.value?.contains(target) === true || submenuBox.value?.contains(target) === true) {
    return;
  }
  dismiss();
}

function attach(): void {
  document.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("blur", dismiss);
  window.addEventListener("resize", dismiss);
  // A scroll anywhere moves the thing the menu is about — the row it was opened on can be somewhere
  // else by the time the scrolling stops — so the menu goes rather than pointing at nothing.
  window.addEventListener("scroll", dismiss, true);
}

function detach(): void {
  document.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("blur", dismiss);
  window.removeEventListener("resize", dismiss);
  window.removeEventListener("scroll", dismiss, true);
}

/**
 * Measures the drawn menu and puts it where it fits.
 *
 * Called on every open, including one that lands on top of a menu that is already up: right-clicking
 * somewhere else is a new question, and the answer is drawn against the new pointer.
 */
async function settle(): Promise<void> {
  if (restoreTo === null) {
    const focused = document.activeElement;
    restoreTo = focused instanceof HTMLElement ? focused : null;
  }
  open.value = null;
  active.value = -1;
  // Drawn at the pointer first — the size is only known once it is in the document — and placed
  // properly on the next tick, which is one frame nobody sees.
  position.value = { left: at.value.x, top: at.value.y };
  await nextTick();
  const element = box.value;
  if (!element) {
    return;
  }
  position.value = placeMenu(
    { x: at.value.x, y: at.value.y },
    { width: element.offsetWidth, height: element.offsetHeight },
    { width: window.innerWidth, height: window.innerHeight },
  );
  element.focus();
}

watch(visible, (shown) => {
  if (shown) {
    void settle();
    attach();
    return;
  }
  detach();
});

watch(at, () => {
  if (visible.value) {
    void settle();
  }
});

onBeforeUnmount(detach);
</script>

<template>
  <Teleport to="body">
    <!--
      `data-menu-layer` is on both lists because a menu is not always the only layer on screen: a
      panel that opened one has to be able to tell a click on its own menu from a click somewhere
      else, and an id is what lets it say so without reaching into what this component draws.
    -->
    <div
      v-if="visible"
      ref="box"
      role="menu"
      tabindex="-1"
      data-menu-layer
      :aria-label="label"
      :aria-activedescendant="rootActiveId"
      :class="boxClass"
      :style="{ left: `${position.left}px`, top: `${position.top}px` }"
      @keydown="onKeydown"
      @contextmenu.prevent
    >
      <ContextMenuRows
        :entries="entries"
        :prefix="ROOT"
        :active="active"
        @hover="(index, entry) => hover(index, entry, 'root')"
        @activate="(index, entry) => activate(index, entry, 'root')"
      />
    </div>

    <!--
      The list a row opened, beside that row. It is a second menu rather than part of the first,
      because that is what it is: the keyboard moves into it, and Escape takes it away first.
    -->
    <div
      v-if="visible && open"
      ref="submenuBox"
      role="menu"
      tabindex="-1"
      data-menu-layer
      :aria-label="label"
      :aria-activedescendant="submenuActiveId"
      :class="boxClass"
      :style="{ left: `${submenuPosition.left}px`, top: `${submenuPosition.top}px` }"
      @keydown="onKeydown"
      @contextmenu.prevent
    >
      <ContextMenuRows
        :entries="open.entry.entries"
        :prefix="SUB"
        :active="subActive"
        @hover="(index, entry) => hover(index, entry, 'sub')"
        @activate="(index, entry) => activate(index, entry, 'sub')"
      />
    </div>
  </Teleport>
</template>
