<script setup lang="ts">
/**
 * The password an SSH connection to a guest needs.
 *
 * Asked before the session is written, so a cancelled question leaves nothing behind. There is no
 * "do not remember" answer: this build stores credentials with the session, and a password it did
 * not keep would be a tab that cannot reconnect. The dialog says where it goes instead of offering
 * a switch that would have to lie about it.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { KeyRound, LoaderCircle } from "@lucide/vue";
import { nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  open: boolean;
  busy: boolean;
  /** `user@address`, so the question says who it is for. */
  target: string;
}>();

const emit = defineEmits<{
  confirm: [password: string];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

const password = ref("");
const field = ref<HTMLInputElement | null>(null);

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      return;
    }
    password.value = "";
    await nextTick();
    field.value?.focus();
  },
);

function submit(): void {
  const value = password.value;
  if (props.busy || value === "") {
    return;
  }
  emit("confirm", value);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(26rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        data-vmware-password-dialog
      >
        <div class="px-4 pt-4">
          <div class="flex items-start gap-2.5">
            <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background">
              <KeyRound :size="14" class="text-muted-foreground" />
            </span>
            <div class="min-w-0">
              <DialogTitle class="text-sm font-medium">{{ t("sessions.vmware.password.title") }}</DialogTitle>
              <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t("sessions.vmware.password.description", { target: props.target }) }}
              </DialogDescription>
            </div>
          </div>

          <div class="mt-3">
            <label class="block text-xs" for="vmware-guest-password">{{ t("sessions.vmware.password.label") }}</label>
            <input
              id="vmware-guest-password"
              ref="field"
              v-model="password"
              type="password"
              autocomplete="new-password"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
              :placeholder="t('sessions.vmware.password.placeholder')"
              @keydown.enter="submit()"
            />
            <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
              {{ t("sessions.vmware.password.hint") }}
            </p>
          </div>
        </div>

        <div class="mt-4 flex justify-end gap-2 border-t border-border px-4 py-3">
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:opacity-60"
            :disabled="props.busy"
            @click="emit('update:open', false)"
          >
            {{ t("common.action.cancel") }}
          </button>
          <button
            type="button"
            data-vmware-password-submit
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy || password === ''"
            @click="submit()"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="animate-spin" />
            {{ t("sessions.vmware.password.connect") }}
          </button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
