<script setup lang="ts">
/**
 * The one question an export asks before it happens: do the passwords go with it?
 *
 * It is a question rather than a setting because the two answers are different files for different
 * journeys — a copy kept for yourself, or a file handed to somebody else — and because the answer
 * that leaks something has to be typed out deliberately rather than inherited from last time.
 *
 * The checkbox is unchecked every time the dialog opens. A remembered "yes" is how a password ends
 * up in a file somebody meant to share.
 */
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";

const props = defineProps<{
  open: boolean;
  /** While the write is running: the dialog stays up and nothing can be pressed twice. */
  busy: boolean;
}>();

const emit = defineEmits<{ confirm: [includeSecrets: boolean]; "update:open": [value: boolean] }>();

const { t } = useI18n();
const includeSecrets = ref(false);

watch(
  () => props.open,
  (open) => {
    if (open) {
      includeSecrets.value = false;
    }
  },
);
</script>

<template>
  <ConfirmDialog
    :open="props.open"
    :title="t('sessions.list.exportTitle')"
    :message="t('sessions.list.exportMessage')"
    :confirm-label="t('sessions.list.exportConfirm')"
    :loading="props.busy"
    @confirm="emit('confirm', includeSecrets)"
    @update:open="emit('update:open', $event)"
  >
    <label class="mt-3 flex items-start gap-2 rounded-md border border-border bg-muted/40 px-3 py-2">
      <input
        v-model="includeSecrets"
        type="checkbox"
        class="mt-0.5 size-3.5 shrink-0 accent-primary"
        :aria-label="t('sessions.list.exportIncludeSecrets')"
      />
      <span class="min-w-0">
        <span class="block text-xs">{{ t("sessions.list.exportIncludeSecrets") }}</span>
        <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
          {{ t("sessions.list.exportIncludeSecretsHint") }}
        </span>
      </span>
    </label>
  </ConfirmDialog>
</template>
