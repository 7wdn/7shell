<script setup lang="ts">
/**
 * The question an agent's call stops on, drawn over whatever is on screen.
 *
 * It is the one dialog in this build that is asked by something other than a person: the call came
 * from another process, the listener is holding it open, and this is the only surface it has. Three
 * rules follow from that:
 *
 * 1. **Refusing is the default.** Focus lands on the button that changes nothing, Escape refuses,
 *    and a click past the dialog is refused too — an approval nobody gave must not be one a stray
 *    key or a stray click produced.
 * 2. **Approving is dangerous, and looks it.** The button that lets a command run is drawn as the
 *    destructive one, whichever tier the call arrived under: a destructive call is approved inside
 *    every tier, and the button is the last chance to see that.
 * 3. **A deadline is shown, and enforced.** The listener refuses the call when it runs out; this
 *    dialog says so before it happens, and closes itself on the same deadline rather than asking
 *    about a call that no longer exists.
 *
 * What it says is the call's own facts — the tool's name, the session, what it would do, and the
 * fingerprint when the host key is the reason. Nothing here is a secret: the subject has been
 * scrubbed on the far side, and an approval is about an action, not a credential.
 */
import { Clock, ShieldAlert, ShieldQuestionMark } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { RISK_LEVEL_LABEL_KEYS } from "@/lib/mcp/labels";
import { useMcpStore } from "@/stores/mcp";
import type { McpApprovalDecision, McpApprovalRequest } from "@/types/mcp";

const { t } = useI18n();
const mcp = useMcpStore();

const request = computed(() => mcp.pendingApproval);
const open = computed(() => request.value !== null);
/** Whether an answer is in flight: both buttons wait, because two answers are one too many. */
const answering = ref(false);
const denyButton = ref<HTMLButtonElement | null>(null);

const problem = computed(() => (mcp.failure ? describeShellError(translate, mcp.failure) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** The clock the countdown is read from, ticked while a question is on screen. */
const now = ref(Date.now());
let ticker: number | undefined;
/** The call whose deadline has already been acted on, so one expiry sends one answer. */
let expired = "";

const remainingMs = computed(() => {
  const current = request.value;
  return current === null ? 0 : Math.max(0, current.expiresAt - now.value);
});

/** Minutes and seconds, because a deadline of two minutes is read faster as `1:58` than as `118s`. */
const countdown = computed(() => {
  const total = Math.ceil(remainingMs.value / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
});

/** What the call is about, as one name: the session's when the listener knows it. */
const session = computed(() => {
  const current = request.value;
  return current === null ? "" : (current.sessionName ?? current.sessionId ?? "");
});

/**
 * The reason, resolved to the two shapes once.
 *
 * Read here rather than narrowed inside the template: `request?.reason.kind === "risk"` in a `v-if`
 * is not a narrowing the compiler carries into the element's other expressions, and two branches
 * that each re-tested the kind would be two places for the union to drift.
 */
const risk = computed(() => (request.value?.reason.kind === "risk" ? request.value.reason.assessment : null));
const hostKey = computed(() => (request.value?.reason.kind === "hostKeyUnknown" ? request.value.reason.key : null));

/**
 * Who is waiting.
 *
 * Two callers share this dialog — a client over MCP and this window's own assistant — and what a
 * person is being asked to trust is not the same for both, so the dialog says which one it is. An
 * absent origin is the listener that only ever had one, which is `mcp`.
 */
const origin = computed(() => (request.value?.origin === "ai" ? "ai" : "mcp"));

/**
 * Focus lands on the button that changes nothing.
 *
 * `preventDefault` because reka-ui would otherwise focus the first focusable thing in the dialog,
 * which is a link or the destructive button depending on what the call carries — and a question
 * answered by a stray Enter is exactly what this rule is for.
 */
function focusDeny(event: Event): void {
  event.preventDefault();
  denyButton.value?.focus();
}

/** Answers the call on screen. A failed answer leaves the question up and says why. */
async function answer(decision: McpApprovalDecision): Promise<void> {
  const current = request.value;
  if (!current || answering.value) {
    return;
  }
  answering.value = true;
  await mcp.answerApproval(current.callId, decision);
  // Nothing is cleared here on failure: the store keeps the question and the reason, which is what
  // the person needs to see before deciding whether to try again.
  answering.value = false;
}

/** The deadline passed: the listener has already refused the call, so the dialog goes with it. */
function expire(current: McpApprovalRequest): void {
  answering.value = true;
  void mcp.answerApproval(current.callId, "denied").then((answered) => {
    answering.value = false;
    if (!answered) {
      mcp.forgetApproval(current.callId);
    }
  });
}

function tick(): void {
  now.value = Date.now();
  const current = request.value;
  if (!current || expired === current.callId || remainingMs.value > 0) {
    return;
  }
  expired = current.callId;
  expire(current);
}

function startTicking(): void {
  stopTicking();
  ticker = window.setInterval(tick, 500);
}

function stopTicking(): void {
  if (ticker !== undefined) {
    window.clearInterval(ticker);
    ticker = undefined;
  }
}

/**
 * A new question is a new deadline, and the keyboard starts where the last one did: on refusing.
 *
 * The dialog is not remounted between calls — the store swaps the request under it — so the focus
 * that `open-auto-focus` gave the first question has to be given to the next one by hand.
 */
watch(
  request,
  (current) => {
    answering.value = false;
    if (current === null) {
      stopTicking();
      return;
    }
    now.value = Date.now();
    startTicking();
    void nextTick(() => denyButton.value?.focus());
  },
  { immediate: true },
);

onBeforeUnmount(stopTicking);

const PRIMARY_DANGER =
  "rounded-md bg-destructive/10 px-3 py-1.5 text-xs text-destructive transition-colors hover:bg-destructive/20";
const SECONDARY = "rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent";
</script>

<template>
  <DialogRoot :open="open">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <!--
        A click past the dialog is a refusal, not a dismissal: the question has to be answered for the
        call to go anywhere, and "clicked outside" is the answer that lets nothing happen.
      -->
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl select-none focus:outline-none"
        @open-auto-focus="focusDeny"
        @interact-outside.prevent="answer('denied')"
        @escape-key-down.prevent="answer('denied')"
      >
        <header class="flex items-start gap-2.5 border-b border-border px-4 py-3">
          <span
            class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-destructive/40 bg-destructive/10"
          >
            <ShieldAlert :size="14" class="text-destructive" />
          </span>
          <div class="min-w-0">
            <DialogTitle class="text-sm font-medium">{{ t("mcp.approval.title") }}</DialogTitle>
            <DialogDescription class="mt-0.5 text-[11px] text-muted-foreground">
              {{ t("mcp.approval.description") }}
            </DialogDescription>
          </div>
        </header>

        <div class="space-y-3 px-4 py-4">
          <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
            <dt class="text-muted-foreground">{{ t("mcp.approval.tool") }}</dt>
            <dd class="font-mono break-all">{{ request?.tool }}</dd>

            <dt class="text-muted-foreground">{{ t("mcp.approval.origin.label") }}</dt>
            <dd>{{ t(origin === "ai" ? "mcp.approval.origin.ai" : "mcp.approval.origin.mcp") }}</dd>

            <template v-if="session">
              <dt class="text-muted-foreground">{{ t("mcp.approval.session") }}</dt>
              <dd class="break-all">{{ session }}</dd>
            </template>

            <dt class="text-muted-foreground">{{ t("mcp.approval.subject") }}</dt>
            <dd class="break-all">{{ request?.subject ?? t("mcp.approval.noSubject") }}</dd>
          </dl>

          <!-- A command, a file write, a container action: the verdict and the rules that fired. -->
          <div v-if="risk" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
            <div class="flex items-center gap-1.5">
              <ShieldAlert :size="13" class="text-destructive" />
              <span class="font-medium">{{ t(RISK_LEVEL_LABEL_KEYS[risk.level]) }}</span>
            </div>
            <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
              {{ t("mcp.approval.reason") }}
            </p>
            <ul v-if="risk.reasons.length > 0" class="mt-1 space-y-0.5">
              <li v-for="reason in risk.reasons" :key="reason.ruleId" class="text-[11px] leading-relaxed">
                {{ t(reason.labelKey) }}
              </li>
            </ul>
          </div>

          <!--
            The host's key, which is the other reason a call stops: the key has not been seen before,
            so the fingerprint is the whole of what a person can decide on.
          -->
          <div v-else-if="hostKey" class="space-y-2 rounded-md border border-border bg-background px-3 py-2">
            <div class="flex items-center gap-1.5 text-xs">
              <ShieldQuestionMark :size="13" class="text-muted-foreground" />
              <span class="font-medium">{{ t("mcp.approval.hostKey.title") }}</span>
            </div>
            <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.approval.hostKey.body") }}</p>
            <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
              <dt class="text-muted-foreground">{{ t("mcp.approval.hostKey.host") }}</dt>
              <dd class="font-mono text-[11px] break-all">{{ hostKey.host }}:{{ hostKey.port }}</dd>
              <dt class="text-muted-foreground">{{ t("mcp.approval.hostKey.keyType") }}</dt>
              <dd class="font-mono text-[11px] break-all">{{ hostKey.keyType }}</dd>
              <dt class="text-muted-foreground">{{ t("mcp.approval.hostKey.fingerprint") }}</dt>
              <dd class="font-mono text-[11px] break-all">{{ hostKey.fingerprint }}</dd>
            </dl>
          </div>

          <p
            v-if="problem"
            data-mcp-approval-problem
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
          >
            <span>{{ t("mcp.approval.failed") }}</span>
            <span class="mt-1 block">{{ problem.message }}</span>
            <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
          </p>

          <!-- The listener's own deadline, said before it happens rather than after. -->
          <p class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <Clock :size="12" />
            {{ remainingMs > 0 ? t("mcp.approval.expiresIn", { time: countdown }) : t("mcp.approval.expired") }}
          </p>
        </div>

        <footer class="flex flex-wrap items-center justify-end gap-2 border-t border-border px-4 py-3">
          <span v-if="answering" class="mr-auto text-[11px] text-muted-foreground">{{
            t("mcp.approval.answering")
          }}</span>
          <button ref="denyButton" type="button" :class="SECONDARY" :disabled="answering" @click="answer('denied')">
            {{ t("mcp.approval.deny") }}
          </button>
          <button type="button" :class="PRIMARY_DANGER" :disabled="answering" @click="answer('approved')">
            {{ t("mcp.approval.approve") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
