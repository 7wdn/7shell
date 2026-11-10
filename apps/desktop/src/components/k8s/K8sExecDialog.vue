<script setup lang="ts">
/**
 * Which container a terminal opens in.
 *
 * A pod can hold several, and picking one is not a detail: a shell in the sidecar is a different
 * machine from a shell in the app. One container is answered without asking, because a menu with one
 * entry is a step that decides nothing.
 */
import { LoaderCircle } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { containerNamesOf, nameOf, type K8sObject } from "@/components/k8s/objects";
import type { ShellError } from "@/lib/backend/errors";
import { describeShellError } from "@/i18n/errors";

const props = defineProps<{
  open: boolean;
  pod: K8sObject | null;
  busy: boolean;
  failure: ShellError | null;
}>();

const emit = defineEmits<{ confirm: [container: string]; "update:open": [value: boolean] }>();

const { t } = useI18n();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const containers = computed(() => (props.pod ? containerNamesOf(props.pod) : []));
const chosen = ref("");

watch(
  () => [props.open, props.pod] as const,
  () => {
    chosen.value = containers.value[0] ?? "";
  },
  { immediate: true },
);

const title = computed(() =>
  props.pod ? t("k8s.exec.title", { pod: nameOf(props.pod) }) : t("k8s.exec.title", { pod: "" }),
);

function submit(): void {
  if (props.busy || chosen.value === "") {
    return;
  }
  emit("confirm", chosen.value);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(26rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="px-4 pt-4">
          <DialogTitle class="text-sm font-medium">{{ title }}</DialogTitle>
          <DialogDescription class="mt-1 text-xs text-muted-foreground">
            {{ t("k8s.exec.message") }}
          </DialogDescription>
        </div>

        <fieldset class="max-h-64 space-y-0.5 overflow-y-auto px-4 pt-3">
          <legend class="sr-only">{{ t("k8s.exec.container") }}</legend>
          <label
            v-for="name in containers"
            :key="name"
            class="flex items-center gap-2 rounded-md px-1 py-1 font-mono text-[11px] transition-colors hover:bg-accent/50"
          >
            <input
              v-model="chosen"
              type="radio"
              :value="name"
              name="k8s-exec-container"
              class="size-3 accent-current"
            />
            {{ name }}
          </label>
        </fieldset>

        <p
          v-if="props.failure"
          role="alert"
          class="mx-4 mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5"
        >
          <span class="block text-xs">{{ describeShellError(translate, props.failure).message }}</span>
          <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">
            {{ describeShellError(translate, props.failure).code }}
          </span>
        </p>

        <footer
          class="mt-3 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="emit('update:open', false)"
          >
            {{ t("common.action.cancel") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy || chosen === ''"
            @click="submit"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ t("k8s.exec.open") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
