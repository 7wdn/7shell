<script setup lang="ts">
/**
 * The one question this interface asks before something happens that cannot be taken back.
 *
 * A dialog rather than a strip that opens inside the row it is about, and the reasons are the ones
 * a strip cannot answer: a question has to be readable without the row's own indentation and the
 * list's own scrolling deciding where it lands, one question should be on screen at a time, and a
 * keyboard has to be able to leave it — Escape cancels, the first Tab reaches the button that
 * changes nothing, and a focus ring that somebody can see says where they are.
 *
 * What it says follows one shape, borrowed from the application this interface is modelled on and
 * worth keeping for its own sake:
 *
 * 1. **The title is the action, named.** "Delete group", not "Are you sure?" — somebody who opened
 *    this by mistake should be able to tell what it is about without reading the sentence.
 * 2. **The message is the consequence.** What is going to happen, in full, including what survives
 *    — a question that only says "delete?" is a question nobody can answer well.
 * 3. **The button is the same verb again.** "Delete group" on the button, never "OK": a button that
 *    names what it does is one nobody presses by reflex.
 *
 * It is deliberately a plain confirmation and not a decision: a decision that changes the shape of
 * what happens next (a host key, a mismatch) has its own dialog, because the answers there are not
 * "yes" and "no".
 *
 * It never closes itself. `open` belongs to the caller, because whether the question should stay on
 * screen while the answer is carried out depends on what the answer does — a delete that can be
 * cancelled is gone at once, while one that walks a directory stays up and says it is working.
 */
import { LoaderCircle, TriangleAlert } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** The action as a noun phrase: "Delete group". */
    title: string;
    /** What happens if this goes ahead, said in full — including what survives. */
    message: string;
    /** The same action again, on the button: "Delete group", never "OK". */
    confirmLabel: string;
    /**
     * A third answer, for the questions that have one: unsaved changes can be saved, thrown away,
     * or left alone, and two buttons would have to misname one of them. Drawn between the two, in
     * the same wash of red as a dangerous confirm — what it does cannot be taken back either.
     */
    discardLabel?: string;
    /** Overridden where the plain "Cancel" would be ambiguous; the default is the shared one. */
    cancelLabel?: string;
    /** What this is about, one line each, when the message cannot name them all. */
    details?: readonly string[];
    /** Styles it as something that destroys: warning icon, and a button that is not the safe one. */
    danger?: boolean;
    /** While the action is running: the dialog stays up and neither button does anything. */
    loading?: boolean;
  }>(),
  { cancelLabel: undefined, discardLabel: undefined, details: undefined, danger: false, loading: false },
);

const emit = defineEmits<{ confirm: []; discard: []; "update:open": [value: boolean] }>();

const { t } = useI18n();

const cancelLabel = computed(() => props.cancelLabel ?? t("common.action.cancel"));

/**
 * The safe button's look, and the other one's.
 *
 * The dangerous one is a wash of red with red text rather than a solid red block: it is the button
 * that must not be pressed by accident, and a page of shouting colour is one people stop reading.
 */
const CANCEL =
  "rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60";
const CONFIRM = "rounded-md px-3 py-1.5 text-xs transition-colors disabled:cursor-not-allowed disabled:opacity-60";
const CONFIRM_PLAIN = "bg-primary text-primary-foreground hover:bg-primary/90";
const CONFIRM_DANGER = "bg-destructive/10 text-destructive hover:bg-destructive/20";

/**
 * Escape and a click outside are the same answer as the cancel button — both are somebody leaving
 * without deciding, which is a cancel. While the action is running there is nothing left to cancel,
 * so the dialog refuses to be dismissed: what is behind it is already happening.
 */
function keepOpenWhileLoading(event: Event): void {
  if (props.loading) {
    event.preventDefault();
  }
}

/**
 * Where the keyboard was before the question was asked.
 *
 * A dialog that closes and drops the focus onto the body has taken the person's place away: the next
 * Tab starts again at the top of the window. This one remembers the control that asked — the trash
 * button, the menu entry, the listing a key was pressed on — and hands it back, whichever way the
 * question was answered. It hands back nothing when that control is gone: a row that was deleted is
 * not a place the keyboard can return to.
 */
let asker: HTMLElement | null = null;

function rememberAsker(): void {
  asker = document.activeElement instanceof HTMLElement ? document.activeElement : null;
}

function handBackFocus(event: Event): void {
  if (!asker || !asker.isConnected) {
    return;
  }
  event.preventDefault();
  asker.focus();
  asker = null;
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @interact-outside="keepOpenWhileLoading"
        @escape-key-down="keepOpenWhileLoading"
        @open-auto-focus="rememberAsker"
        @close-auto-focus="handBackFocus"
      >
        <div class="px-4 pt-4">
          <div class="flex items-start gap-2.5">
            <span
              class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border"
              :class="props.danger ? 'border-destructive/40 bg-destructive/10' : 'border-border bg-background'"
            >
              <TriangleAlert :size="14" :class="props.danger ? 'text-destructive' : 'text-warning'" />
            </span>
            <div class="min-w-0">
              <DialogTitle class="text-sm font-medium">{{ props.title }}</DialogTitle>
              <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ props.message }}
              </DialogDescription>
            </div>
          </div>

          <!-- What the action is about, when there is more of it than a sentence can name. -->
          <ul
            v-if="props.details && props.details.length > 0"
            class="mt-3 max-h-32 space-y-0.5 overflow-y-auto rounded-md border border-border bg-muted/40 px-3 py-2"
          >
            <li
              v-for="detail in props.details"
              :key="detail"
              class="font-mono text-[11px] break-all text-muted-foreground"
            >
              {{ detail }}
            </li>
          </ul>

          <!-- Extra choices — a checkbox, a field — belong to the caller, in the order it draws them. -->
          <slot />
        </div>

        <footer
          class="mt-4 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button type="button" :class="CANCEL" :disabled="props.loading" @click="emit('update:open', false)">
            {{ cancelLabel }}
          </button>
          <button
            v-if="props.discardLabel"
            type="button"
            :class="[CONFIRM, CONFIRM_DANGER]"
            :disabled="props.loading"
            @click="emit('discard')"
          >
            {{ props.discardLabel }}
          </button>
          <button
            type="button"
            :class="[CONFIRM, props.danger ? CONFIRM_DANGER : CONFIRM_PLAIN]"
            :disabled="props.loading"
            @click="emit('confirm')"
          >
            <LoaderCircle v-if="props.loading" :size="12" class="mr-1.5 inline animate-spin" />
            {{ props.confirmLabel }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
