<script setup lang="ts">
/**
 * Where a toast is drawn: one line, centred at the foot of the window, above everything else.
 *
 * Two live regions rather than one, because a region's politeness is fixed when it is created — a
 * single region cannot wait politely for a receipt and interrupt for a failure. Both stay in the
 * document so what gets announced is the text arriving, not a region appearing; the message is
 * drawn into whichever one matches its tier.
 *
 * The line is only interactive where it carries an action: everything else lets the click through
 * to whatever is underneath, so a toast never blocks a control it happens to be covering.
 */
import { computed } from "vue";

import { toastInterrupts, useToast } from "@/composables/useToast";
import { cn } from "@/lib/utils";

const { message, tier, action, visible } = useToast();

/** A refusal or a failure interrupts; anything else waits for a pause. */
const interrupting = computed(() => toastInterrupts(tier.value));

/**
 * A solid line, in the colours of whatever it is saying.
 *
 * Opaque rather than tinted: a message that matters arrives over whatever was already on screen —
 * a terminal, a form, a table — and a translucent strip over that is a sentence read twice, once
 * through itself. An ordinary receipt is drawn the way the reference draws it, light on dark; a
 * failure is drawn in the colour of a failure.
 */
const lineClass = computed(() =>
  cn(
    "pointer-events-auto max-w-[min(90vw,48rem)] rounded-lg px-4 py-2 text-sm break-words whitespace-pre-wrap shadow-lg",
    interrupting.value ? "bg-destructive text-destructive-foreground" : "bg-foreground text-background",
  ),
);

/** The button on the line, in whatever the line is written in. */
const actionClass = computed(() =>
  cn(
    "ml-3 shrink-0 rounded border px-2 py-0.5 text-xs font-medium transition-colors",
    interrupting.value
      ? "border-destructive-foreground/40 hover:bg-destructive-foreground/15"
      : "border-background/40 hover:bg-background/15",
  ),
);
</script>

<template>
  <div class="pointer-events-none fixed inset-x-0 bottom-6 z-[60] flex flex-col items-center px-4">
    <div aria-live="polite" aria-atomic="true" class="flex max-w-full justify-center">
      <p v-if="visible && !interrupting" :class="lineClass">
        <span>{{ message }}</span>
        <button v-if="action" type="button" :class="actionClass" @click="action.onClick()">
          {{ action.label }}
        </button>
      </p>
    </div>

    <div aria-live="assertive" aria-atomic="true" class="flex max-w-full justify-center">
      <p v-if="visible && interrupting" role="alert" :class="lineClass">
        <span>{{ message }}</span>
        <button v-if="action" type="button" :class="actionClass" @click="action.onClick()">
          {{ action.label }}
        </button>
      </p>
    </div>
  </div>
</template>
