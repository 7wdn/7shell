<script setup lang="ts">
/**
 * The question an already-open session asks, wherever the open came from.
 *
 * The store owns the question and the answer, so the session menu and the tab strip's picker share
 * one prompt rather than each drawing its own. The checkbox is the whole difference from a plain
 * confirmation: an answer that is the same every time is worth keeping.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const terminals = useTerminalsStore();

const question = computed(() => terminals.reopenQuestion);
const remember = computed({
  get: () => terminals.reopenQuestion?.remember ?? false,
  set: (value: boolean) => {
    if (terminals.reopenQuestion) {
      terminals.reopenQuestion.remember = value;
    }
  },
});
</script>

<template>
  <ConfirmDialog
    v-if="question"
    :open="true"
    :title="t('sessions.reopen.title')"
    :message="t('sessions.reopen.message', { name: question.session.name })"
    :confirm-label="t('sessions.reopen.confirm')"
    @confirm="terminals.confirmReopen()"
    @update:open="terminals.declineReopen()"
  >
    <label class="mt-3 flex items-start gap-2 text-xs">
      <input v-model="remember" type="checkbox" class="mt-0.5 size-3.5 accent-primary" />
      <span>
        <span>{{ t("sessions.reopen.remember") }}</span>
        <span class="mt-0.5 block text-[11px] text-muted-foreground">
          {{ t("sessions.reopen.rememberNote") }}
        </span>
      </span>
    </label>
  </ConfirmDialog>
</template>
