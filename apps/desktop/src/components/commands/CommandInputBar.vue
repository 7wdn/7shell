<script setup lang="ts">
/**
 * The line at the bottom of a terminal: type one, press Enter, it runs.
 *
 * This is the bar for the command somebody did not save: the panel under it is for the lines worth
 * keeping, and this is for `tail -f` and `cd`. Enter sends, because that is what Enter does in the
 * shell above it — and the input clears afterwards for the same reason a shell prompt does.
 *
 * The history is the same list the panel shows, which is the point: a line typed here is offered
 * back there, and a line sent from a saved command is offered back here. Two lists would be two
 * answers to "what did I just run".
 */
import { History, Send } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { describeErrorFields } from "@/i18n/errors";
import { useCommandsStore } from "@/stores/commands";

const { t } = useI18n();
const commands = useCommandsStore();

const line = ref("");
const showHistory = ref(false);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const recent = computed(() => commands.history.slice(0, 12));

/** Why the last line did not reach the shell, when it reached nothing. */
const problem = computed(() => {
  const failed = commands.sendFailures.at(-1);
  return failed ? describeErrorFields(translate, failed) : null;
});

async function send(): Promise<void> {
  const text = line.value;
  if (text.trim() === "") {
    return;
  }

  // Cleared before the answer comes back, the way a prompt would: a line that hangs waiting for a
  // slow connection is a line somebody types twice.
  line.value = "";
  showHistory.value = false;
  await commands.sendLiteral(text);
}

function reuse(entry: string): void {
  line.value = entry;
  showHistory.value = false;
}
</script>

<template>
  <div class="relative flex shrink-0 items-center gap-2 border-t border-border bg-card/30 px-3 py-1">
    <input
      v-model="line"
      spellcheck="false"
      autocomplete="off"
      class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-0.5 font-mono text-[11px] focus:border-primary focus:outline-none"
      :placeholder="t('commands.input.placeholder')"
      :aria-label="t('commands.input.label')"
      @keydown.enter.prevent="send"
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
      @click="showHistory = !showHistory"
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
      @click="send"
    >
      <Send :size="11" />
      {{ t("commands.send") }}
    </button>

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
