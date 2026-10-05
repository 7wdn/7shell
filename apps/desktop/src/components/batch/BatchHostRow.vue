<script setup lang="ts">
/**
 * One host's row in a batch: what it answered, and what it said.
 *
 * The row is deliberately quiet until it has something to say. A host that has not answered yet is
 * a name and an address — the honest state — rather than a spinner per row, which for ten hosts is
 * ten things that look like work happening everywhere at once.
 *
 * Output is folded away rather than shown: a batch is read by scanning exit codes, and the one
 * host that failed is the one whose output is worth unfolding. Standard error is kept apart from
 * standard output because they answer different questions — "what did it print" and "why did it
 * complain" — and merging them loses which is which.
 */
import { Check, ChevronDown, ChevronRight, CircleAlert, LoaderCircle, MinusCircle, RotateCw } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { describeErrorFields } from "@/i18n/errors";
import type { BatchRow } from "@/stores/batch";
import { rowFailed } from "@/stores/batch";

const props = defineProps<{ row: BatchRow; retrying: boolean }>();
const emit = defineEmits<{ retry: [sessionId: string] }>();

const { t } = useI18n();
const open = ref(false);

const failure = computed(() => (props.row.result?.outcome.kind === "failed" ? props.row.result.outcome.error : null));
const ran = computed(() => (props.row.result?.outcome.kind === "ran" ? props.row.result.outcome : null));
const failed = computed(() => rowFailed(props.row));

/**
 * Why a host has nothing to show, in the active language.
 *
 * Through the same renderer every other failure goes through, so a code whose message key this
 * build does not know falls back to something quotable here too rather than to a raw key.
 */
const described = computed(() => (failure.value ? describeErrorFields(t, failure.value) : null));

const duration = computed(() => {
  const ms = props.row.result?.durationMs;
  if (ms === undefined) {
    return "";
  }
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
});

const hasOutput = computed(() => {
  const outcome = ran.value;
  return outcome !== null && (outcome.stdout !== "" || outcome.stderr !== "");
});

/** Whether there is anything behind the row to unfold: output, or a failure's own explanation. */
const expandable = computed(() => hasOutput.value || described.value !== null);
</script>

<template>
  <li class="border-b border-border/60 last:border-b-0">
    <div class="flex items-center gap-2 px-3 py-2 text-xs">
      <span class="grid size-5 shrink-0 place-items-center">
        <LoaderCircle v-if="row.status === 'running'" :size="13" class="animate-spin text-muted-foreground" />
        <MinusCircle v-else-if="row.status === 'pending'" :size="13" class="text-muted-foreground/50" />
        <CircleAlert v-else-if="failed" :size="13" class="text-destructive" />
        <Check v-else :size="13" class="text-emerald-500" />
      </span>

      <button
        type="button"
        class="flex min-w-0 flex-1 items-center gap-2 text-left"
        :class="expandable ? 'cursor-pointer' : 'cursor-default'"
        :aria-expanded="expandable ? open : undefined"
        @click="expandable && (open = !open)"
      >
        <component
          :is="open ? ChevronDown : ChevronRight"
          v-if="expandable"
          :size="12"
          class="shrink-0 text-muted-foreground"
        />
        <span class="min-w-0 truncate font-medium">{{ row.sessionName }}</span>
        <span class="shrink-0 font-mono text-[11px] text-muted-foreground">
          {{ row.session.user }}@{{ row.session.host }}:{{ row.session.port }}
        </span>
      </button>

      <span
        v-if="described"
        class="max-w-[24rem] truncate text-[11px] text-destructive"
        :title="described.detail ?? described.code"
      >
        {{ described.message }}
      </span>
      <span
        v-else-if="ran"
        class="shrink-0 font-mono text-[11px]"
        :class="failed ? 'text-destructive' : 'text-muted-foreground'"
      >
        {{ ran.exitCode === null ? t("batch.result.noStatus") : t("batch.result.exit", { code: ran.exitCode }) }}
      </span>
      <span class="shrink-0 font-mono text-[11px] text-muted-foreground/70">{{ duration }}</span>

      <button
        v-if="row.status === 'done' && failed"
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
        :disabled="retrying"
        :title="t('batch.action.retryOne')"
        :aria-label="t('batch.action.retryOne')"
        @click="emit('retry', row.sessionId)"
      >
        <RotateCw :size="12" />
      </button>
    </div>

    <div v-if="open && ran" class="space-y-2 px-3 pb-3 pl-10">
      <div v-if="ran.stdout" class="rounded-md border border-border bg-background">
        <p class="border-b border-border px-2 py-1 text-[10px] tracking-wide text-muted-foreground uppercase">
          {{ t("batch.result.stdout") }}
        </p>
        <pre class="max-h-64 overflow-auto px-2 py-1.5 font-mono text-[11px] whitespace-pre-wrap">{{ ran.stdout }}</pre>
      </div>
      <div v-if="ran.stderr" class="rounded-md border border-destructive/40 bg-destructive/5">
        <p class="border-b border-destructive/30 px-2 py-1 text-[10px] tracking-wide text-muted-foreground uppercase">
          {{ t("batch.result.stderr") }}
        </p>
        <pre class="max-h-64 overflow-auto px-2 py-1.5 font-mono text-[11px] whitespace-pre-wrap">{{ ran.stderr }}</pre>
      </div>
    </div>

    <!--
      A host that never ran unfolds into the code and the backend's own detail rather than into
      output it does not have. The code is here rather than only in the tooltip because it is the
      part a bug report can quote.
    -->
    <div v-if="open && described" class="px-3 pb-3 pl-10">
      <div class="rounded-md border border-destructive/40 bg-destructive/5 px-2 py-1.5">
        <p class="font-mono text-[10px] text-muted-foreground">{{ described.code }}</p>
        <p class="mt-1 text-[11px] leading-relaxed">{{ described.detail ?? described.message }}</p>
      </div>
    </div>
  </li>
</template>
