<script setup lang="ts">
/**
 * The assistant, docked beside the workspace.
 *
 * A panel rather than a tab, for the reason the monitor is one: the conversation is about whatever
 * is on screen, and a tab would be a place somebody has to leave the terminal to visit. It is on the
 * right because that is the edge the other instrument lives on, and it is put away rather than
 * unmounted for the same reason a sidebar is — the state belongs to the window, not to the drawing.
 *
 * Four things are deliberate:
 *
 * 1. **Nothing is asked without a provider and a key.** The panel says what is missing and offers the
 *    way to the settings page; it does not send a request that would only come back refused.
 * 2. **The preview is the sent text.** `ContextPreview` draws the string the store just read, and
 *    `send` reads again immediately before it asks — so what travelled is what was on screen.
 * 3. **A run's rows outlive a look elsewhere.** The store subscribes, not this component, so putting
 *    the panel away does not lose the answer that is still arriving.
 * 4. **An approval is asked by the frame.** A tool call that needs a person's answer stops on the
 *    global dialog, which is already drawn over everything; this panel draws a card and nothing else.
 */
import { Bot, CircleAlert, X } from "@lucide/vue";
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Composer from "@/components/assistant/Composer.vue";
import ContextPreview from "@/components/assistant/ContextPreview.vue";
import Message from "@/components/assistant/Message.vue";
import ToolCard from "@/components/assistant/ToolCard.vue";
import Select from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { SETUP_GUIDE_KEYS, STOP_REASON_LABEL_KEYS, STOP_REASON_UNKNOWN_KEY } from "@/lib/assistant/labels";
import { useAssistantStore } from "@/stores/assistant";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const assistant = useAssistantStore();
const preferences = usePreferencesStore();
const terminals = useTerminalsStore();

const scrolling = ref<HTMLDivElement | null>(null);
/** Whether the plain "the chat could not be cleared" line is on screen. */
const clearFailed = ref(false);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const options = computed(() =>
  assistant.providers.map((provider) => ({
    value: provider.id,
    label: provider.name,
    detail: provider.model,
  })),
);

/** Which provider is chosen: the pinned one, or the first — the same answer the store sends to. */
const chosen = computed(() => {
  const pinned = preferences.aiDefaultProviderId;
  if (pinned !== null && options.value.some((option) => option.value === pinned)) {
    return pinned;
  }
  return assistant.activeProvider?.id ?? "";
});

function chooseProvider(id: string): void {
  preferences.setAiDefaultProviderId(id === "" ? null : id);
  // Written at once rather than with the settings page's Apply: it is a choice about the next
  // question, and the next question may be asked from this panel without that page ever opening.
  void preferences.commit();
  void assistant.refreshProviders();
}

const problem = computed(() => (assistant.failure ? describeShellError(translate, assistant.failure) : null));
const providerProblem = computed(() =>
  assistant.providerFailure ? describeShellError(translate, assistant.providerFailure) : null,
);

/** The run's own failure, in the language of the error catalog. */
const runProblem = computed(() => {
  const event = assistant.runError;
  if (!event) {
    return null;
  }
  return {
    message: translate(event.messageKey, { ...event.messageParams }),
    code: event.code,
    retryable: event.retryable,
  };
});

/** The vault cannot say whether a key is held, which is not the same as "there is none". */
const keyUnknown = computed(
  () => assistant.providersLoaded && assistant.activeProvider !== null && assistant.activeProvider.hasKey === null,
);

/** The guidance the panel shows instead of a question that cannot be asked, or nothing. */
const setup = computed(() => (assistant.setupProblem === null ? null : SETUP_GUIDE_KEYS[assistant.setupProblem]));

const stopLabel = computed(() => {
  const reason = assistant.stopReason;
  if (reason === null) {
    return "";
  }
  return t(STOP_REASON_LABEL_KEYS[reason] ?? STOP_REASON_UNKNOWN_KEY);
});

const usageText = computed(() => {
  const usage = assistant.usage;
  if (!usage) {
    return "";
  }
  const parts = [t("ai.usage.tokens", { input: usage.inputTokens, output: usage.outputTokens })];
  if (usage.cacheReadTokens !== undefined) {
    parts.push(t("ai.usage.cached", { cached: usage.cacheReadTokens }));
  }
  if (assistant.turns !== null) {
    parts.push(t("ai.usage.turns", { count: assistant.turns }));
  }
  return parts.join(" · ");
});

/**
 * A signature of everything that adds height to the list.
 *
 * Watched rather than the entries themselves: streaming appends to the same object, so a watcher on
 * the array would fire once for a message and never again for the text inside it.
 */
const heightSignature = computed(() => {
  let characters = 0;
  for (const entry of assistant.entries) {
    if (entry.kind === "text" || entry.kind === "thinking") {
      characters += entry.text.length;
    } else if (entry.kind === "tool") {
      characters += entry.preview.length;
    }
  }
  return `${assistant.entries.length}:${characters}`;
});

/** Follows the answer while it is being written, but only for somebody already at the bottom. */
function followAnswer(): void {
  const box = scrolling.value;
  if (!box) {
    return;
  }
  const distance = box.scrollHeight - box.scrollTop - box.clientHeight;
  if (distance > 120) {
    return;
  }
  void nextTick(() => {
    box.scrollTop = box.scrollHeight;
  });
}

watch(heightSignature, followAnswer);

onMounted(() => {
  assistant.refreshContext();
  void assistant.refreshProviders();
});

// The lines a question would carry belong to the tab on screen, so a switch of tabs reads again.
watch(
  () => terminals.activeTabId,
  () => assistant.refreshContext(),
);

async function clear(): Promise<void> {
  clearFailed.value = false;
  clearFailed.value = !(await assistant.reset());
}

function openSettings(): void {
  terminals.openSettings("ai");
}
</script>

<template>
  <aside
    data-panel-frame
    data-assistant-panel
    class="flex w-[22rem] shrink-0 flex-col overflow-hidden border-l border-border bg-card/20"
    :aria-label="t('ai.title')"
  >
    <header class="shrink-0 border-b border-border px-3 py-[var(--panel-header-y)]">
      <div class="flex items-center gap-2">
        <Bot :size="13" class="shrink-0 text-muted-foreground" />
        <h2 class="min-w-0 flex-1 truncate text-xs font-medium">{{ t("ai.title") }}</h2>
        <button
          type="button"
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('ai.close')"
          :aria-label="t('ai.close')"
          @click="assistant.setOpen(false)"
        >
          <X :size="13" />
        </button>
      </div>

      <div class="mt-1.5 flex items-center gap-2">
        <span class="shrink-0 text-[10px] tracking-wide text-muted-foreground uppercase">
          {{ t("ai.provider.label") }}
        </span>
        <div class="min-w-0 flex-1">
          <Select
            :model-value="chosen"
            :options="options"
            size="compact"
            :placeholder="t('ai.setup.noProvider.title')"
            :aria-label="t('ai.provider.label')"
            @update:model-value="chooseProvider"
          />
        </div>
      </div>
    </header>

    <div ref="scrolling" class="min-h-0 flex-1 space-y-3 overflow-y-auto px-3 py-2">
      <!--
        The way to the settings, before anything is asked. The panel says which half is missing —
        a provider, or a key — because they are fixed on the same page but by different fields.
      -->
      <div v-if="setup" data-assistant-setup class="space-y-1.5 rounded-md border border-border bg-muted/20 px-3 py-2">
        <p class="text-xs font-medium">{{ t(setup.title) }}</p>
        <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t(setup.body) }}</p>
        <button
          type="button"
          class="rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent"
          @click="openSettings()"
        >
          {{ t("ai.setup.action") }}
        </button>
      </div>
      <p v-else-if="keyUnknown" class="text-[11px] leading-relaxed text-muted-foreground">
        {{ t("ai.setup.locked") }}
      </p>

      <p
        v-if="providerProblem"
        data-assistant-provider-problem
        class="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-[11px] select-text"
      >
        <span>{{ providerProblem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ providerProblem.code }}</span>
      </p>

      <div v-if="!assistant.hasMessages" class="space-y-1 rounded-md border border-dashed border-border px-3 py-4">
        <p class="text-xs font-medium">{{ t("ai.empty.title") }}</p>
        <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("ai.empty.body") }}</p>
      </div>

      <template v-for="entry in assistant.entries" :key="entry.id">
        <Message
          v-if="entry.kind === 'user'"
          role="user"
          :text="entry.text"
          :context-lines="entry.context === '' ? 0 : entry.context.split('\n').length"
        />
        <Message v-else-if="entry.kind === 'text'" role="assistant" :text="entry.text" :streaming="entry.streaming" />
        <Message
          v-else-if="entry.kind === 'thinking'"
          role="thinking"
          :text="entry.text"
          :streaming="entry.streaming"
        />
        <ToolCard v-else-if="entry.kind === 'tool'" :block="entry" />
      </template>

      <!-- The run's own failure: what the provider said, kept until the next question. -->
      <div
        v-if="runProblem"
        data-assistant-run-problem
        class="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 select-text"
      >
        <p class="flex items-start gap-1.5 text-[11px] font-medium">
          <CircleAlert :size="12" class="mt-0.5 shrink-0 text-destructive" />
          <span>{{ t("ai.error.title") }}</span>
        </p>
        <p class="mt-1 pl-[18px] text-[11px] leading-relaxed">{{ runProblem.message }}</p>
        <p class="mt-0.5 pl-[18px] font-mono text-[10px] text-muted-foreground">{{ runProblem.code }}</p>
        <p v-if="runProblem.retryable" class="mt-0.5 pl-[18px] text-[10px] text-muted-foreground">
          {{ t("ai.error.retryable") }}
        </p>
      </div>

      <p
        v-if="problem"
        data-assistant-problem
        class="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 text-[11px] select-text"
      >
        <span>{{ problem.message }}</span>
        <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
      </p>
    </div>

    <ContextPreview
      :enabled="preferences.aiContextEnabled"
      :lines="preferences.aiContextLines"
      :context="assistant.context"
      :line-count="assistant.contextLines"
      @toggle="(value) => preferences.setAiContextEnabled(value)"
      @lines="(value) => preferences.setAiContextLines(value)"
      @refresh="assistant.refreshContext()"
    />

    <footer class="shrink-0 space-y-1.5 border-t border-border px-3 py-2">
      <!-- What the last answer cost, and how it ended: two facts about a run that is no longer on. -->
      <div v-if="usageText || stopLabel" class="flex items-baseline gap-2 text-[10px] text-muted-foreground">
        <span v-if="stopLabel" class="shrink-0">{{ stopLabel }}</span>
        <span class="min-w-0 flex-1 truncate text-right font-mono">{{ usageText }}</span>
      </div>
      <p v-if="clearFailed" class="text-[10px] text-destructive">
        {{ t("ai.composer.clearFailed") }}
        <span v-if="problem" class="font-mono"> · {{ problem.code }}</span>
      </p>

      <Composer
        :running="assistant.running"
        :sending="assistant.sending"
        :can-clear="assistant.hasMessages"
        :clearing="assistant.clearing"
        :blocked="assistant.setupProblem !== null"
        @send="(text) => assistant.send(text)"
        @stop="assistant.cancel()"
        @clear="clear()"
      />
    </footer>
  </aside>
</template>
