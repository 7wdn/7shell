<script setup lang="ts">
/**
 * The list above the command line: what could come next, and one line about each.
 *
 * It opens upward because the line it belongs to sits at the bottom of the window — the same reason
 * the history folds out above the bar instead of pushing it down. Nothing here decides what is
 * offered: the candidates arrive already ordered, and this layer draws them, moves through them and
 * hands back the row somebody took.
 *
 * The rows are options in a listbox rather than buttons, and the row the arrows are on is named by
 * `aria-activedescendant`: the keyboard never leaves the input, so the caret stays where it was while
 * somebody is choosing — the same shape the context menu has, for the same reason.
 */
import { Bookmark, CornerDownRight, FolderOpen, History, Shell, Terminal } from "@lucide/vue";
import { nextTick, ref, watch, type Component } from "vue";
import { useI18n } from "vue-i18n";

import type { CompletionCandidate, CompletionKind } from "@/lib/commands/completion";
import type { MatchRange } from "@/lib/commands/match";
import { cn } from "@/lib/utils";

const props = defineProps<{
  candidates: readonly CompletionCandidate[];
  /** Which row the keyboard is on. */
  active: number;
  /** What this list's ids start with, so the input can name the row it is on. */
  listId: string;
}>();

const emit = defineEmits<{
  (event: "hover", index: number): void;
  (event: "take", index: number): void;
}>();

const { t } = useI18n();

/** The icon a row wears: which list it came from, readable before the label is. */
const ICONS: Readonly<Record<CompletionKind, Component>> = {
  command: Terminal,
  builtin: Shell,
  // The arrow the editor in this window uses for "something you can call": a subcommand is a word
  // that belongs to the program in front of it rather than to the shell.
  subcommand: CornerDownRight,
  saved: Bookmark,
  history: History,
  path: FolderOpen,
};

const ICON_COLOURS: Readonly<Record<CompletionKind, string>> = {
  command: "text-completion-command",
  builtin: "text-completion-builtin",
  // The amber of "a word the shell or a program owns", which is what a subcommand is.
  subcommand: "text-completion-builtin",
  saved: "text-completion-saved",
  history: "text-completion-history",
  path: "text-completion-path",
};

const list = ref<HTMLElement | null>(null);

/**
 * A piece of text, split where the prefix answered.
 *
 * The ranges come from the engine, which is the only thing that knows what was matched and how — a
 * second search here would be a second answer to that question, and the two would eventually differ.
 */
function pieces(text: string, ranges: readonly MatchRange[]): { text: string; hit: boolean }[] {
  const parts: { text: string; hit: boolean }[] = [];
  let cursor = 0;
  for (const range of ranges) {
    if (range.start > cursor) {
      parts.push({ text: text.slice(cursor, range.start), hit: false });
    }
    parts.push({ text: text.slice(range.start, range.end), hit: true });
    cursor = range.end;
  }
  if (cursor < text.length) {
    parts.push({ text: text.slice(cursor), hit: false });
  }
  return parts;
}

// The row the keyboard is on is kept in view: a list that scrolls away from the arrows is a list that
// hides what is about to be taken.
watch(
  () => props.active,
  async () => {
    await nextTick();
    list.value?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  },
);
</script>

<template>
  <div
    :id="listId"
    ref="list"
    role="listbox"
    :aria-label="t('commands.completion.label')"
    class="absolute bottom-full left-0 z-30 mb-1 max-h-[min(280px,calc(100vh-32px))] w-full min-w-[280px] max-w-[min(760px,calc(100vw-24px))] overflow-y-auto rounded-md border border-border bg-card py-1 shadow-lg"
    @mousedown.prevent
  >
    <div
      v-for="(candidate, index) in candidates"
      :id="`${listId}-${index}`"
      :key="`${candidate.kind}-${candidate.insert}`"
      role="option"
      :aria-selected="index === active"
      :class="cn('flex h-7 cursor-default items-center gap-2 px-2.5', index === active && 'bg-accent')"
      @mousemove="emit('hover', index)"
      @click="emit('take', index)"
    >
      <component :is="ICONS[candidate.kind]" :size="13" class="shrink-0" :class="ICON_COLOURS[candidate.kind]" />
      <span class="max-w-[60%] shrink-0 truncate font-mono text-xs">
        <span
          v-for="(piece, at) in pieces(candidate.label, candidate.labelRanges)"
          :key="at"
          :class="piece.hit ? 'font-medium text-primary' : ''"
          >{{ piece.text }}</span
        >
      </span>
      <span v-if="candidate.detail" class="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
        <span
          v-for="(piece, at) in pieces(candidate.detail, candidate.detailRanges)"
          :key="at"
          :class="piece.hit ? 'font-medium text-primary' : ''"
          >{{ piece.text }}</span
        >
      </span>
    </div>
  </div>
</template>
