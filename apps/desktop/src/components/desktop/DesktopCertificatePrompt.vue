<script setup lang="ts">
/**
 * The certificate question before a remote desktop authenticates.
 *
 * RDP has no host key; the TLS certificate is what stands in for one. An unknown certificate is a
 * question and a changed one is a stop, drawn as the different things they are — a rebuilt Windows
 * machine is a real thing that happens, so replacing is reachable, but it is never the recommended
 * action.
 */
import { ShieldAlert, ShieldQuestionMark } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import type { DesktopDecision } from "@/stores/terminals";

const props = defineProps<{ decision: DesktopDecision }>();
const emit = defineEmits<{ trust: []; cancel: [] }>();

const { t } = useI18n();
const cancelButton = ref<HTMLButtonElement | null>(null);

const mismatch = computed(() => props.decision.outcome.kind === "certificateMismatch");
const certificate = computed(() => props.decision.outcome.certificate);
const trustedFingerprint = computed(() =>
  props.decision.outcome.kind === "certificateMismatch" ? props.decision.outcome.trusted.fingerprint : "",
);

const PRIMARY =
  "rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90";
const SECONDARY = "rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent";
const DANGEROUS =
  "rounded-md border border-destructive/40 px-3 py-1.5 text-xs text-destructive transition-colors hover:bg-destructive/10";

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
              {{ mismatch ? t("desktop.certificate.mismatchTitle") : t("desktop.certificate.unknownTitle") }}
            </DialogTitle>
            <DialogDescription class="mt-0.5 text-[11px] text-muted-foreground">
              {{ t("desktop.certificate.opening", { name: decision.session.name }) }}
            </DialogDescription>
          </div>
        </header>

        <div class="space-y-3 px-4 py-4">
          <p class="text-xs leading-relaxed" :class="mismatch ? 'text-destructive' : 'text-muted-foreground'">
            {{ mismatch ? t("desktop.certificate.mismatchBody") : t("desktop.certificate.unknownBody") }}
          </p>

          <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
            <dt class="text-muted-foreground">{{ t("desktop.certificate.host") }}</dt>
            <dd class="font-mono break-all">{{ certificate.host }}:{{ certificate.port }}</dd>
          </dl>

          <div v-if="mismatch" class="grid gap-2 sm:grid-cols-2">
            <div class="rounded-md border border-border bg-background px-3 py-2">
              <p class="text-[11px] text-muted-foreground">{{ t("desktop.certificate.trusted") }}</p>
              <p class="mt-1 font-mono text-[11px] break-all">{{ trustedFingerprint }}</p>
            </div>
            <div class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
              <p class="text-[11px] text-muted-foreground">{{ t("desktop.certificate.presented") }}</p>
              <p class="mt-1 font-mono text-[11px] break-all">{{ certificate.fingerprint }}</p>
            </div>
          </div>
          <div v-else class="rounded-md border border-border bg-background px-3 py-2">
            <p class="text-[11px] text-muted-foreground">{{ t("desktop.certificate.presented") }}</p>
            <p class="mt-1 font-mono text-[11px] break-all">{{ certificate.fingerprint }}</p>
          </div>

          <p
            v-if="mismatch"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px] leading-relaxed"
          >
            {{ t("desktop.certificate.replaceWarning") }}
          </p>
        </div>

        <footer class="flex flex-wrap justify-end gap-2 border-t border-border px-4 py-3">
          <button ref="cancelButton" type="button" :class="mismatch ? PRIMARY : SECONDARY" @click="emit('cancel')">
            {{ t("desktop.certificate.cancel") }}
          </button>
          <button type="button" :class="mismatch ? DANGEROUS : PRIMARY" @click="emit('trust')">
            {{ mismatch ? t("desktop.certificate.replace") : t("desktop.certificate.trust") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
