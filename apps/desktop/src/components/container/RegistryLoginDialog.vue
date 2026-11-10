<script setup lang="ts">
/**
 * Signing in to an image registry.
 *
 * The password goes one way: into `SecretInput` and out through the command's stdin on the Rust
 * side. It is never stored by the interface, and it is cleared from this form the moment the answer
 * arrives — a dialog that kept it would be a password sitting in a reactive object for as long as
 * the pane lived.
 *
 * What the person gets for signing in is a credential the runtime keeps; this view only reports
 * whether it worked.
 */
import { computed, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import { supportsInsecure, useContainerStore } from "@/stores/container";
import type { ContainerRuntime } from "@/types/container";

import ContainerDialog from "./ContainerDialog.vue";

const props = defineProps<{
  open: boolean;
  tabId: string;
  runtime: ContainerRuntime;
  /** Prefilled when the login was prompted by a push that was refused. */
  presetRegistry?: string;
}>();

const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const containers = useContainerStore();

const form = reactive({ registry: "", username: "", password: "", insecure: false });
const busy = ref(false);
const failure = ref<string | null>(null);

const canSkipTls = computed(() => supportsInsecure(props.runtime));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    form.registry = props.presetRegistry ?? "";
    form.username = "";
    form.password = "";
    form.insecure = false;
    busy.value = false;
    failure.value = null;
  },
);

async function submit(): Promise<void> {
  if (busy.value) {
    return;
  }
  if (form.username.trim() === "" || form.password === "") {
    failure.value = t("container.registry.missing");
    return;
  }

  busy.value = true;
  failure.value = null;
  try {
    await containers.registryLogin(props.tabId, {
      registry: form.registry.trim() === "" ? undefined : form.registry.trim(),
      username: form.username.trim(),
      // Inwards only: there is no type for a secret travelling back out.
      secret: { value: form.password },
      insecure: canSkipTls.value && form.insecure ? true : undefined,
    });
    form.password = "";
    emit("update:open", false);
  } catch (error) {
    form.password = "";
    failure.value = describeShellError(translate, asShellError(error, "container_registry_login")).message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <ContainerDialog
    :open="props.open"
    :title="t('container.registry.title')"
    :busy="busy"
    @update:open="emit('update:open', $event)"
  >
    <div class="flex flex-col gap-2.5">
      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.registry.registry") }}</span>
        <input
          v-model="form.registry"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
          :placeholder="t('container.registry.registryPlaceholder')"
        />
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.registry.username") }}</span>
        <input
          v-model="form.username"
          type="text"
          spellcheck="false"
          autocomplete="username"
          class="rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none"
        />
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.registry.password") }}</span>
        <input
          v-model="form.password"
          type="password"
          autocomplete="current-password"
          class="rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none"
          @keydown.enter.prevent="submit()"
        />
      </label>

      <label v-if="canSkipTls" class="flex items-center gap-1.5 text-[11px]">
        <input v-model="form.insecure" type="checkbox" class="size-3 accent-[var(--primary)]" />
        {{ t("container.transfer.skipTlsVerify") }}
      </label>

      <p v-if="failure" class="text-[11px] text-destructive select-text">{{ failure }}</p>
    </div>

    <template #footer>
      <button
        type="button"
        class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-40"
        :disabled="busy"
        @click="emit('update:open', false)"
      >
        {{ t("common.action.cancel") }}
      </button>
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
        :disabled="busy"
        @click="submit()"
      >
        {{ t("container.registry.submit") }}
      </button>
    </template>
  </ContainerDialog>
</template>
