<script setup lang="ts">
/**
 * The two host key decisions, drawn as the different things they are.
 *
 * `hostKeyUnknown` is a question, and the answer that connects is a button someone has to press:
 * focusing happens on the button that changes nothing, so a stray Enter cancels rather than
 * trusting whatever just answered.
 *
 * `hostKeyMismatch` is not a question but a stop. The trusted fingerprint and the one being
 * presented are shown together, the safe action is the one styled as primary, and the action that
 * forgets the old key is styled and labelled so it cannot be read as the recommended choice —
 * while still being reachable, because a rebuilt server is a real thing that happens.
 *
 * Neither dialog closes on a click outside: a decision that was made by clicking past the dialog
 * is a decision nobody made.
 */
import { ShieldAlert, ShieldQuestionMark } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import type { HostKeyDecision } from "@/stores/terminals";

const props = defineProps<{ decision: HostKeyDecision }>();
const emit = defineEmits<{ trust: []; replace: []; cancel: [] }>();

const { t } = useI18n();
const cancelButton = ref<HTMLButtonElement | null>(null);

const mismatch = computed(() => props.decision.kind === "hostKeyMismatch");
const trustedFingerprint = computed(() =>
  props.decision.kind === "hostKeyMismatch" ? props.decision.trusted.fingerprint : "",
);

const PRIMARY =
  "rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90";
const SECONDARY = "rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent";
const DANGEROUS =
  "rounded-md border border-destructive/40 px-3 py-1.5 text-xs text-destructive transition-colors hover:bg-destructive/10";

/** Focus goes to the action that does nothing, which is the only one a mis-pressed key may take. */
function focusCancel(event: Event): void {
  event.preventDefault();
  cancelButton.value?.focus();
}
</script>

<template>
  <DialogRoot :open="true">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @open-auto-focus="focusCancel"
        @interact-outside.prevent
        @escape-key-down="emit('cancel')"
      >
        <header class="flex items-start gap-2.5 border-b border-border px-4 py-3">
          <span
            class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border"
            :class="mismatch ? 'border-destructive/40 bg-destructive/10' : 'border-border bg-background'"
          >
            <ShieldAlert v-if="mismatch" :size="14" class="text-destructive" />
            <ShieldQuestionMark v-else :size="14" class="text-muted-foreground" />
          </span>
          <div class="min-w-0">
            <DialogTitle class="text-sm font-medium">
              {{ mismatch ? t("sessions.hostKey.mismatchTitle") : t("sessions.hostKey.unknownTitle") }}
            </DialogTitle>
            <DialogDescription class="mt-0.5 text-[11px] text-muted-foreground">
              {{ t("sessions.hostKey.opening", { name: decision.session.name }) }}
            </DialogDescription>
          </div>
        </header>

        <div class="space-y-3 px-4 py-4">
          <p class="text-xs leading-relaxed" :class="mismatch ? 'text-destructive' : 'text-muted-foreground'">
            {{ mismatch ? t("sessions.hostKey.mismatchBody") : t("sessions.hostKey.unknownBody") }}
          </p>

          <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
            <dt class="text-muted-foreground">{{ t("sessions.hostKey.host") }}</dt>
            <dd class="font-mono break-all">{{ decision.key.host }}:{{ decision.key.port }}</dd>
            <dt class="text-muted-foreground">{{ t("sessions.hostKey.keyType") }}</dt>
            <dd class="font-mono break-all">{{ decision.key.keyType }}</dd>
          </dl>

          <div v-if="mismatch" class="grid gap-2 sm:grid-cols-2">
            <div class="rounded-md border border-border bg-background px-3 py-2">
              <p class="text-[11px] text-muted-foreground">{{ t("sessions.hostKey.fingerprintTrusted") }}</p>
              <p class="mt-1 font-mono text-[11px] break-all">{{ trustedFingerprint }}</p>
            </div>
            <div class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
              <p class="text-[11px] text-muted-foreground">{{ t("sessions.hostKey.fingerprintPresented") }}</p>
              <p class="mt-1 font-mono text-[11px] break-all">{{ decision.key.fingerprint }}</p>
            </div>
          </div>
          <div v-else class="rounded-md border border-border bg-background px-3 py-2">
            <p class="text-[11px] text-muted-foreground">{{ t("sessions.hostKey.fingerprintPresented") }}</p>
            <p class="mt-1 font-mono text-[11px] break-all">{{ decision.key.fingerprint }}</p>
          </div>

          <p
            v-if="mismatch"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px] leading-relaxed"
          >
            {{ t("sessions.hostKey.replaceWarning") }}
          </p>
        </div>

        <footer class="flex flex-wrap justify-end gap-2 border-t border-border px-4 py-3">
          <button ref="cancelButton" type="button" :class="mismatch ? PRIMARY : SECONDARY" @click="emit('cancel')">
            {{ t("sessions.hostKey.cancel") }}
          </button>
          <button v-if="mismatch" type="button" :class="DANGEROUS" @click="emit('replace')">
            {{ t("sessions.hostKey.replace") }}
          </button>
          <button v-else type="button" :class="PRIMARY" @click="emit('trust')">
            {{ t("sessions.hostKey.trust") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
