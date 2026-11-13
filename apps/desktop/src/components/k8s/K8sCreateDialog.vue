<script setup lang="ts">
/**
 * Creating one object from a manifest.
 *
 * The manifest is text rather than a form, because every kind's fields differ and a form per kind is
 * a component per kind — the opposite of what the descriptor table is for. The template is a starting
 * point that the person is expected to edit; nothing here validates it, because the cluster's own
 * validation is the only answer that counts, and its message is what the dialog shows back.
 */
import { LoaderCircle } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import type { ResourceDescriptor } from "@/components/k8s/resources";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";

const props = defineProps<{
  open: boolean;
  descriptor: ResourceDescriptor | null;
  /** The namespace the manifest is aimed at; empty asks the manifest to name one. */
  namespace: string;
  busy: boolean;
  failure: ShellError | null;
}>();

const emit = defineEmits<{ confirm: [manifest: string]; "update:open": [value: boolean] }>();

const { t } = useI18n();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const manifest = ref("");

/** Each opening starts from the kind's template, aimed at the namespace on screen. */
watch(
  () => [props.open, props.descriptor, props.namespace] as const,
  () => {
    if (!props.open) {
      return;
    }
    manifest.value = props.descriptor?.createTemplate?.(props.namespace) ?? "";
  },
  { immediate: true },
);

/**
 * What the kind is called in the title and on the button.
 *
 * A kind the cluster defined has no message key of its own, so its own word is used; everything
 * around it stays translated.
 */
const kindLabel = computed(() =>
  props.descriptor ? (props.descriptor.kindLabel ?? t(props.descriptor.labelKey)) : "",
);

const title = computed(() => t("k8s.create.title", { kind: kindLabel.value }));

const submitLabel = computed(() => t("k8s.create.submit", { kind: kindLabel.value }));

/**
 * Whether the manifest has nowhere to be created.
 *
 * A namespaced kind is posted to a namespaced collection, and the all-namespaces list has no such
 * URL; the dialog says so up front rather than letting the server answer "namespace is required".
 */
const needsNamespace = computed(() => props.descriptor?.namespaced === true && props.namespace.trim() === "");

const canSubmit = computed(() => manifest.value.trim() !== "" && !props.busy && !needsNamespace.value);

function submit(): void {
  if (!canSubmit.value) {
    return;
  }
  emit("confirm", manifest.value);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[calc(100vh-4rem)] w-[min(44rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="shrink-0 px-4 pt-4">
          <DialogTitle class="text-sm font-medium">{{ title }}</DialogTitle>
          <DialogDescription class="mt-1 text-xs text-muted-foreground">
            {{ t("k8s.create.message") }}
          </DialogDescription>
        </div>

        <div class="flex min-h-0 flex-1 flex-col gap-1 px-4 pt-3">
          <label class="text-[11px] text-muted-foreground" for="k8s-create-manifest">
            {{ t("k8s.create.yaml") }}
          </label>
          <textarea
            id="k8s-create-manifest"
            v-model="manifest"
            spellcheck="false"
            class="min-h-64 flex-1 resize-none rounded-md border border-border bg-background px-2 py-1.5 font-mono text-[11px] leading-relaxed focus:outline-none"
            :aria-label="t('k8s.create.yaml')"
          />
        </div>

        <p v-if="needsNamespace" class="shrink-0 px-4 pt-2 text-[11px] text-muted-foreground">
          {{ t("k8s.create.namespace") }}
        </p>

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
          class="mt-3 flex shrink-0 flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
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
            :disabled="!canSubmit"
            @click="submit"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ submitLabel }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
