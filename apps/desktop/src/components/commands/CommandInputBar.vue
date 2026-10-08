<script setup lang="ts">
/**
 * The line at the bottom of a terminal: type one, press Enter, it runs.
 *
 * This is the bar for the command somebody did not save: the panel under it is for the lines worth
 * keeping, and this is for `tail -f` and `cd`. Enter sends, because that is what Enter does in the
 * shell above it — and the input clears afterwards for the same reason a shell prompt does.
 *
 * The line also completes, the way the editor in this window would: what is offered is what this
 * application knows — the commands it has a word about, the lines somebody saved, the ones they have
 * run — and it is offered beside the line rather than typed for them. Tab takes the row the keyboard
 * is on, and nothing else does: Enter keeps meaning "run this", because a shell is a place where the
 * difference between the line somebody typed and a line that is almost it is the whole story.
 *
 * The history is the same list the panel shows, which is the point: a line typed here is offered
 * back there, and a line sent from a saved command is offered back here. Two lists would be two
 * answers to "what did I just run".
 */
import { History, Send } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import CompletionMenu from "@/components/commands/CompletionMenu.vue";
import { useCommandCompletion } from "@/components/commands/useCommandCompletion";
import { useRemotePaths } from "@/components/commands/useRemotePaths";
import { describeErrorFields } from "@/i18n/errors";
import { COMMAND_CATALOG, descriptionKeyFor } from "@/lib/commands/catalog";
import { buildCommandCompletions } from "@/lib/commands/completion";
import { subcommandKeyFor, subcommandsOf } from "@/lib/commands/subcommands";
import { matchesShortcut } from "@/lib/shortcuts/binding";
import { bindingFor } from "@/lib/shortcuts/registry";
import { useAppStore } from "@/stores/app";
import { useCommandsStore } from "@/stores/commands";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore, type TerminalTab } from "@/stores/terminals";

const { t, te } = useI18n();
const app = useAppStore();
const commands = useCommandsStore();
const files = useFilesStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();

const line = ref("");
const showHistory = ref(false);
const field = ref<HTMLInputElement | null>(null);
/** True while an input method is composing: those keys are writing, not choosing. */
const composing = ref(false);

/** The id this instance's list carries, so the input can name the row it is on. */
let opened = 0;
const listId = `command-completion-${(opened += 1)}`;

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const recent = computed(() => commands.history.slice(0, 12));

/**
 * The host's own directories, read over the file panel's channel.
 *
 * The channel is the panel the terminal is drawing under itself: the same connection to the same
 * host, and no second one opened behind anybody's back. A terminal whose panel is closed has nothing
 * to read, and then only absolute paths are answered.
 *
 * Reading happens while a list is being built rather than before it: the engine asks what is in a
 * directory, this answers with what has been read or `null`, and a `null` starts the read — so the
 * first keystroke that mentions a directory gets nothing and the answer arrives while somebody keeps
 * typing. When it lands the list is rebuilt, but only for the line the question was asked about: an
 * answer that arrives after somebody has typed on belongs to a question that is no longer on screen.
 */
const remote = useRemotePaths(() => {
  const handle = terminals.activeTerminalHandle();
  if (handle === null) {
    return null;
  }

  const tab = terminals.tabs.find(
    (entry): entry is TerminalTab => entry.kind === "terminal" && entry.handle === handle,
  );
  const panel = tab?.panel?.handle ?? null;
  const browser = panel === null ? null : files.browser(panel);
  return browser === null ? null : { handle: browser.handle, cwd: browser.shellPath };
});

const { candidates, active, chosen, visible, refresh, open, hide, dismiss, move, take } = useCommandCompletion(
  line,
  (text, caret) =>
    buildCommandCompletions({
      line: text,
      caret,
      sources: {
        saved: commands.groups.flatMap((group) => group.commands),
        history: commands.history,
        catalog: COMMAND_CATALOG,
      },
      describe: (name) => (te(descriptionKeyFor(name)) ? t(descriptionKeyFor(name)) : null),
      subcommands: subcommandsOf,
      describeSubcommand: (command, word) => {
        const key = subcommandKeyFor(command, word);
        return te(key) ? t(key) : null;
      },
      kindLabel: (kind) => t(`commands.completion.kind.${kind}`),
      paths: {
        cwd: remote.base,
        entries: (directory) => {
          const asked = `${caret}@${text}`;
          remote.warm(directory, () => {
            if (`${caretOf()}@${line.value}` === asked) {
              refresh(caretOf());
            }
          });
          return remote.entries(directory);
        },
      },
    }),
);

/** Why the last line did not reach the shell, when it reached nothing. */
const problem = computed(() => {
  const failed = commands.sendFailures.at(-1);
  return failed ? describeErrorFields(translate, failed) : null;
});

/** Where the caret is: what a candidate replaces up to, and where the list is built from. */
function caretOf(): number {
  return field.value?.selectionStart ?? line.value.length;
}

/**
 * Somebody is pointing at a row.
 *
 * The arrow keys say it with the keyboard and the pointer says it by resting on a row; either way it
 * is a choice, and the highlight that follows is what Enter and Tab will act on. The list opens with
 * nothing pointed at, so a highlight on screen always means exactly that.
 */
function pointAt(index: number): void {
  active.value = index;
  chosen.value = true;
}

function onInput(): void {
  if (composing.value || !preferences.commandCompletion) {
    return;
  }
  refresh(caretOf());
}

function onKeydown(event: KeyboardEvent): void {
  // While an input method is composing, the keyboard is writing: Enter and Tab are how its own
  // candidate list is chosen, and taking them here would swallow what somebody is typing. Tab is
  // still held back from leaving the field, because it never leaves this field.
  if (composing.value || event.isComposing) {
    if (event.key === "Tab") {
      event.preventDefault();
    }
    return;
  }

  // With the list turned off the bar is a plain line: its keys are the browser's again, Tab included.
  if (!preferences.commandCompletion) {
    return;
  }

  if (matchesShortcut(event, bindingFor("commands.completionTrigger"), app.platform)) {
    event.preventDefault();
    showHistory.value = false;
    open(caretOf());
    return;
  }

  if (matchesShortcut(event, bindingFor("commands.completionAccept"), app.platform)) {
    event.preventDefault();
    if (visible.value) {
      takeRow(active.value);
    } else {
      open(caretOf());
    }
    return;
  }

  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    if (!visible.value) {
      return;
    }
    event.preventDefault();
    move(event.key === "ArrowDown" ? 1 : -1);
    return;
  }

  if (event.key === "Escape" && visible.value) {
    event.preventDefault();
    dismiss(caretOf());
  }
}

/**
 * Taking a row: the line becomes what the row inserts, and the caret lands after it.
 *
 * The list is fed by the line it just changed, so it stays consistent while somebody keeps typing —
 * which is also why the caret has to be put back by hand: the value was replaced outside the field's
 * own editing, and the field would otherwise leave the caret where the old text ended.
 *
 * A word is followed by the next step rather than by silence: taking `systemctl` opens what comes
 * after it, which is the moment somebody has most clearly said what they are doing. A whole line ends
 * the sentence instead — there is no next word to offer.
 */
function takeRow(index: number): void {
  active.value = index;
  const row = candidates.value[index];
  const applied = take(caretOf());
  if (applied === null) {
    return;
  }

  line.value = applied.text;
  void nextTick(() => {
    field.value?.setSelectionRange(applied.caret, applied.caret);
    field.value?.focus();
    if (row !== undefined && !row.wholeLine) {
      open(applied.caret);
    }
  });
}

function onCompositionStart(): void {
  composing.value = true;
  hide();
}

function onCompositionEnd(): void {
  composing.value = false;
  if (preferences.commandCompletion) {
    refresh(caretOf());
  }
}

// Turning the switch off while the list is open takes it away rather than leaving it stranded: the
// setting is a decision about what is on screen, and it applies at once.
watch(
  () => preferences.commandCompletion,
  (on) => {
    if (!on) {
      hide();
    }
  },
);

function toggleHistory(): void {
  showHistory.value = !showHistory.value;
  if (showHistory.value) {
    hide();
  }
}

/**
 * What Enter does.
 *
 * A row somebody pointed at goes *into the line*, not to the shell: a command name is the beginning of
 * a line — `chmod` chosen out of `ch` is half a command, and running it the moment it was chosen is
 * running something nobody finished writing. The line on screen is the answer to "what will run", so
 * Enter puts the row there and stops; the next Enter, with nothing pointed at any more, runs it.
 *
 * With nothing pointed at, Enter runs exactly what was typed — which is what the bar is for, and what
 * keeps a list that opened on its own (`ls` and a highlighted `lsblk`) from running anything.
 */
function send(): void {
  if (chosen.value && visible.value) {
    takeRow(active.value);
    return;
  }

  void sendLine(line.value);
}

/** The button beside the line: the mouse's way of running what is in it, whatever the list is showing. */
function sendTyped(): void {
  void sendLine(line.value);
}

async function sendLine(text: string): Promise<void> {
  if (text.trim() === "") {
    return;
  }

  // Cleared before the answer comes back, the way a prompt would: a line that hangs waiting for a
  // slow connection is a line somebody types twice.
  line.value = "";
  showHistory.value = false;
  dismiss(0);
  await commands.sendLiteral(text);
}

function reuse(entry: string): void {
  line.value = entry;
  showHistory.value = false;
  dismiss(entry.length);
}
</script>

<template>
  <div class="relative flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1">
    <input
      ref="field"
      v-model="line"
      spellcheck="false"
      autocomplete="off"
      class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-0.5 font-mono text-[11px] focus:border-primary focus:outline-none"
      :placeholder="t('commands.input.placeholder')"
      :aria-label="t('commands.input.label')"
      role="combobox"
      aria-autocomplete="list"
      :aria-expanded="visible"
      :aria-controls="listId"
      :aria-activedescendant="visible && chosen ? `${listId}-${active}` : undefined"
      @input="onInput"
      @keydown="onKeydown"
      @keydown.enter.prevent="send"
      @compositionstart="onCompositionStart"
      @compositionend="onCompositionEnd"
      @blur="hide"
    />

    <span
      v-if="problem"
      class="max-w-[22rem] truncate text-[10px] text-destructive"
      :title="problem.detail ?? problem.code"
    >
      {{ problem.message }}
    </span>

    <button
      type="button"
      class="flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] transition-colors"
      :class="showHistory ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent hover:text-foreground'"
      :aria-expanded="showHistory"
      :title="t('commands.history')"
      @click="toggleHistory"
    >
      <History :size="11" />
      {{ t("commands.history") }}
    </button>

    <button
      type="button"
      class="flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] transition-colors"
      :class="
        line.trim() === ''
          ? 'cursor-not-allowed text-muted-foreground/50'
          : 'bg-primary text-primary-foreground hover:bg-primary/90'
      "
      :disabled="line.trim() === '' || commands.sending"
      :title="t('commands.send')"
      :aria-label="t('commands.input.send')"
      @click="sendTyped"
    >
      <Send :size="11" />
      {{ t("commands.send") }}
    </button>

    <!--
      The candidates, above the line rather than under it: the bar sits at the bottom of the window,
      and a list that pushed it up would move what somebody is typing at.
    -->
    <CompletionMenu
      v-if="visible"
      :candidates="candidates"
      :active="chosen ? active : -1"
      :list-id="listId"
      @hover="pointAt"
      @take="takeRow"
    />

    <!--
      The history folds out above the bar rather than pushing it down: the bar is where the caret
      is, and a list that moves the input while somebody is reaching for it is a list that eats
      keystrokes.
    -->
    <div
      v-if="showHistory"
      class="absolute bottom-full left-0 z-30 mb-1 max-h-64 w-full overflow-auto rounded-md border border-border bg-card shadow-lg"
    >
      <div class="flex items-center justify-between border-b border-border px-2 py-1">
        <span class="text-[10px] tracking-wide text-muted-foreground uppercase">{{ t("commands.history") }}</span>
        <button
          type="button"
          class="rounded-sm px-1 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="commands.clearHistory()"
        >
          {{ t("commands.historyClear") }}
        </button>
      </div>
      <button
        v-for="(entry, index) in recent"
        :key="`${entry.at}-${index}`"
        type="button"
        class="block w-full truncate px-2 py-1 text-left font-mono text-[11px] transition-colors hover:bg-accent/50"
        :title="entry.command"
        @click="reuse(entry.command)"
      >
        {{ entry.command }}
      </button>
      <p v-if="recent.length === 0" class="px-2 py-2 text-[11px] text-muted-foreground">
        {{ t("commands.historyEmpty") }}
      </p>
    </div>
  </div>
</template>
