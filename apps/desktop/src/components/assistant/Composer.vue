<script setup lang="ts">
/**
 * The line a question is asked in, and the three things that can happen to one.
 *
 * The keyboard is the whole point of it: Enter sends and Shift+Enter starts a new line, because a
 * chat box is not a document and somebody who has to reach for the mouse to send has already lost
 * the thing they were about to say. The text is kept until the send is accepted — a question that
 * vanished into a failure would be one the person has to type again.
 *
 * Stopping replaces sending rather than standing beside it: the two are never both useful, and two
 * buttons that swap places are harder to miss than two that sit next to each other.
 */
import { Eraser, Library, LoaderCircle, SendHorizontal, Square } from "@lucide/vue";
import { nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const props = withDefaults(
  defineProps<{
    /** A run is producing: the send button becomes the stop button. */
    running: boolean;
    /** A send is in flight: nothing may be typed into a second one. */
    sending: boolean;
    /** There is a chat to clear. */
    canClear: boolean;
    /** True while the clear is in flight, so the button says so rather than doing nothing twice. */
    clearing: boolean;
    /** No provider or no key: a question would not go anywhere, so the line says why instead. */
    blocked: boolean;
  }>(),
  {},
);

const emit = defineEmits<{
  send: [text: string];
  stop: [];
  clear: [];
  /** Somebody wants the server-content picker, which only the panel owns. */
  "open-content": [];
}>();

const { t } = useI18n();
const text = ref("");
const box = ref<HTMLTextAreaElement | null>(null);

const INPUT_CLASS =
  "w-full resize-none rounded-md border border-border bg-background px-2.5 py-1.5 text-xs leading-relaxed text-foreground transition-colors placeholder:text-muted-foreground hover:bg-accent/20 focus-visible:border-primary focus-visible:ring-1 focus-visible:ring-ring/40 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-60 select-text";

function submit(): void {
  const trimmed = text.value.trim();
  if (trimmed === "" || props.running || props.sending || props.blocked) {
    return;
  }
  emit("send", trimmed);
  text.value = "";
  void nextTick(() => box.value?.focus());
}

/**
 * Enter sends, unless it is arriving with Shift — and nothing is taken while an input method is
 * composing, which is the one case where Enter belongs to the text rather than to the form.
 */
function onKeydown(event: KeyboardEvent): void {
  if (event.key !== "Enter" || event.shiftKey || event.isComposing) {
    return;
  }
  event.preventDefault();
  submit();
}

/** A run that ends leaves the line ready for the next question. */
watch(
  () => props.running,
  (running) => {
    if (!running) {
      void nextTick(() => box.value?.focus());
    }
  },
);

/**
 * Appends rendered text to whatever is already typed.
 *
 * A rendered prompt arrives while somebody may have written half a question, so it is added below
 * rather than replacing the line, separated by a blank line — the insertion is a block of text that
 * came from a server, and gluing it to the person's own sentence would make the two indistinguishable.
 *
 * The caret is put after the insertion: the text is a draft, and the next thing typed belongs at
 * its end.
 */
function append(addition: string): void {
  const block = addition.trim();
  if (block === "") {
    return;
  }
  const existing = text.value.replace(/\s+$/, "");
  text.value = existing === "" ? block : `${existing}\n\n${block}`;
  void nextTick(() => {
    const element = box.value;
    if (!element) {
      return;
    }
    element.focus();
    element.setSelectionRange(text.value.length, text.value.length);
  });
}

defineExpose({ append });
</script>

<template>
  <div class="space-y-1.5">
    <textarea
      ref="box"
      v-model="text"
      rows="3"
      :class="INPUT_CLASS"
      :placeholder="t('ai.composer.placeholder')"
      :aria-label="t('ai.composer.placeholder')"
      :disabled="running || sending"
      @keydown="onKeydown"
    ></textarea>

    <div class="flex items-center gap-1.5">
      <p class="min-w-0 flex-1 truncate text-[10px] text-muted-foreground">{{ t("ai.composer.hint") }}</p>

      <button
        type="button"
        data-assistant-content
        class="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
        :title="t('ai.content.open')"
        :aria-label="t('ai.content.open')"
        @click="emit('open-content')"
      >
        <Library :size="13" />
      </button>

      <button
        type="button"
        class="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
        :title="t('ai.composer.clear')"
        :aria-label="t('ai.composer.clear')"
        :disabled="!canClear || running || sending || clearing"
        @click="emit('clear')"
      >
        <LoaderCircle v-if="clearing" :size="13" class="animate-spin" />
        <Eraser v-else :size="13" />
      </button>

      <button
        v-if="running"
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent"
        @click="emit('stop')"
      >
        <Square :size="11" />
        {{ t("ai.composer.stop") }}
      </button>
      <button
        v-else
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
        :disabled="sending || blocked || text.trim() === ''"
        @click="submit()"
      >
        <LoaderCircle v-if="sending" :size="11" class="animate-spin" />
        <SendHorizontal v-else :size="11" />
        {{ t("ai.composer.send") }}
      </button>
    </div>
  </div>
</template>
