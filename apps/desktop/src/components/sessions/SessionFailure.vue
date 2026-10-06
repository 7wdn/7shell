<script setup lang="ts">
/**
 * The mark a session that would not connect carries, and the reason behind it.
 *
 * Two answers in one place because they are the same answer at two depths: the row says something
 * went wrong at a glance, and the click says what. The reason is somebody else's sentence — a
 * driver's, a socket's, a server's — so it is shown as it arrived, in a field that can be selected
 * and copied rather than as a paragraph that has been reworded on the way.
 *
 * It is a popover rather than a dialog or a panel: the question is about one row, and the answer
 * belongs next to it. A dialog would blank the screen to say nine words, and a page in the
 * workspace would be the thing this replaced.
 */
import { TriangleAlert } from "@lucide/vue";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";

const props = defineProps<{
  /** What the attempt failed with, as the envelope arrived. */
  readonly failure: ShellError;
  /** What to call the session in the popover's heading. */
  readonly name: string;
}>();

const emit = defineEmits<{ dismiss: [] }>();

const { t } = useI18n();

const described = computed(() =>
  describeShellError((key, params) => (params ? t(key, params) : t(key)), props.failure),
);

/** The whole answer: the localized sentence, the code, and whatever the far side added. */
const detail = computed(() =>
  described.value.detail ? `${described.value.message}\n\n${described.value.detail}` : described.value.message,
);
</script>

<template>
  <PopoverRoot>
    <PopoverTrigger as-child>
      <button
        type="button"
        data-session-failure
        class="grid size-5 shrink-0 place-items-center rounded-sm text-warning transition-colors hover:bg-warning/10 focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
        :title="t('sessions.list.failureHint', { name: props.name })"
        :aria-label="t('sessions.list.failureHint', { name: props.name })"
        @mousedown.stop
        @click.stop
      >
        <TriangleAlert :size="13" />
      </button>
    </PopoverTrigger>
    <PopoverPortal>
      <PopoverContent
        side="top"
        align="start"
        :side-offset="6"
        :collision-padding="8"
        class="z-50 w-96 max-w-[calc(100vw-2rem)] rounded-lg border border-border bg-card p-3 shadow-xl outline-none"
        @click.stop
        @mousedown.stop
      >
        <div class="flex items-start justify-between gap-2">
          <p class="text-xs font-medium">{{ t("sessions.list.failureTitle") }}</p>
          <button
            type="button"
            class="shrink-0 rounded-sm px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="emit('dismiss')"
          >
            {{ t("sessions.list.failureClear") }}
          </button>
        </div>
        <textarea
          readonly
          data-session-failure-detail
          class="mt-2 max-h-40 min-h-20 w-full resize-none rounded-md border border-border bg-muted/30 p-2 font-mono text-[11px] leading-4 text-muted-foreground outline-none"
          :value="detail"
          @focus="($event.target as HTMLTextAreaElement).select()"
        ></textarea>
        <p class="mt-1.5 font-mono text-[10px] text-muted-foreground">{{ described.code }}</p>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
