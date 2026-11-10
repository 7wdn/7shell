<script setup lang="ts">
/**
 * One object in full: its readings, and the document the server sent.
 *
 * Read-only on purpose. Editing a manifest means posting arbitrary YAML at a cluster, which is a
 * different feature with a different risk — what this drawer is for is answering "what is this and
 * why is it like that", and the YAML tab is how somebody copies the object somewhere else.
 *
 * A Secret is the one object whose values are not printed here. Its keys are, because which
 * credential an object holds is the useful half; the material would travel no further than a
 * screenshot, which is exactly how a secret leaves a machine.
 */
import { Copy, LoaderCircle, X } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { age, createdAtOf, nameOf, namespaceOf, textAt, type K8sObject } from "@/components/k8s/objects";
import type { ResourceDescriptor } from "@/components/k8s/resources";
import { redactedForDisplay, toYaml } from "@/components/k8s/yaml";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";
import { cn } from "@/lib/utils";

const props = defineProps<{
  open: boolean;
  object: K8sObject | null;
  descriptor: ResourceDescriptor | null;
  /** The object's own events, read by the pane when what is open is a pod. */
  events: readonly K8sObject[];
  eventsFailure: ShellError | null;
  eventsLoading: boolean;
}>();

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const tab = ref<"detail" | "yaml">("detail");

/** Opening another object starts again on its readings, not on the tab the last one was left on. */
watch(
  () => props.object,
  () => {
    tab.value = "detail";
  },
);

const shown = computed(() => props.object);

const sections = computed(() => props.descriptor?.detail ?? []);

const yaml = computed(() => (props.object ? toYaml(redactedForDisplay(props.object)) : ""));

const title = computed(() => {
  if (!props.object || !props.descriptor) {
    return "";
  }
  return `${t(props.descriptor.labelKey)} / ${namespaceOf(props.object) || t("k8s.detail.cluster")} / ${nameOf(props.object)}`;
});

const copied = ref(false);

/** Writes the YAML out. Writing to the clipboard is the one direction the webview can do itself. */
async function copyYaml(): Promise<void> {
  try {
    await navigator.clipboard.writeText(yaml.value);
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    copied.value = false;
  }
}

function eventLine(obj: K8sObject): string {
  const type = textAt(obj, "type") || "Normal";
  const reason = textAt(obj, "reason");
  const message = textAt(obj, "message");
  return `${type} ${reason}: ${message}`;
}

function eventWhen(obj: K8sObject): string {
  return age(textAt(obj, "lastTimestamp") || createdAtOf(obj));
}
</script>

<template>
  <aside
    v-if="props.open && shown"
    class="absolute inset-y-0 right-0 z-20 flex w-[min(30rem,100%)] flex-col border-l border-border bg-card shadow-xl"
    :aria-label="title"
  >
    <header class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
      <h2 class="min-w-0 flex-1 truncate font-mono text-[11px]" :title="title">{{ title }}</h2>
      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('common.action.close')"
        :aria-label="t('common.action.close')"
        @click="emit('close')"
      >
        <X :size="13" />
      </button>
    </header>

    <div class="flex shrink-0 items-center gap-1 border-b border-border px-2" role="tablist">
      <button
        type="button"
        role="tab"
        :aria-selected="tab === 'detail'"
        :class="
          cn(
            'border-b-2 px-2 py-1.5 text-xs transition-colors',
            tab === 'detail'
              ? 'border-primary text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground',
          )
        "
        @click="tab = 'detail'"
      >
        {{ t("k8s.detail.tabs.detail") }}
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="tab === 'yaml'"
        :class="
          cn(
            'border-b-2 px-2 py-1.5 text-xs transition-colors',
            tab === 'yaml'
              ? 'border-primary text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground',
          )
        "
        @click="tab = 'yaml'"
      >
        {{ t("k8s.detail.tabs.yaml") }}
      </button>
      <button
        v-if="tab === 'yaml'"
        type="button"
        class="ml-auto flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        @click="copyYaml()"
      >
        <Copy :size="11" />
        {{ copied ? t("k8s.yaml.copied") : t("k8s.yaml.copy") }}
      </button>
    </div>

    <div v-if="tab === 'detail'" class="min-h-0 flex-1 overflow-auto px-3 py-2">
      <section v-for="section in sections" :key="section.key" class="mb-3">
        <h3 class="mb-1 text-xs font-medium text-muted-foreground">{{ t(section.labelKey) }}</h3>
        <dl>
          <div
            v-for="field in section.fields"
            :key="field.key"
            class="flex gap-3 border-b border-border/50 py-1 last:border-b-0"
          >
            <dt class="w-28 shrink-0 text-[11px] text-muted-foreground">{{ t(field.labelKey) }}</dt>
            <dd class="min-w-0 flex-1 font-mono text-[11px] break-all whitespace-pre-wrap select-text">
              {{ field.value(shown) || "—" }}
            </dd>
          </div>
        </dl>
      </section>

      <section v-if="props.descriptor?.kind === 'Pod'" class="mb-3">
        <h3 class="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          {{ t("k8s.detail.sections.events") }}
          <LoaderCircle v-if="props.eventsLoading" :size="10" class="animate-spin" />
        </h3>
        <p v-if="props.eventsFailure" class="text-[11px] text-destructive">
          {{ describeShellError(translate, props.eventsFailure).message }}
        </p>
        <p v-else-if="props.events.length === 0" class="text-[11px] text-muted-foreground">
          {{ t("k8s.events.none") }}
        </p>
        <ul v-else class="space-y-1">
          <li v-for="event in props.events" :key="nameOf(event)" class="flex gap-2 text-[11px]">
            <span class="w-10 shrink-0 font-mono text-muted-foreground">{{ eventWhen(event) }}</span>
            <span class="min-w-0 flex-1 break-words">{{ eventLine(event) }}</span>
          </li>
        </ul>
      </section>

      <p v-if="props.descriptor?.redactData" class="text-[11px] text-muted-foreground">
        {{ t("k8s.detail.redacted") }}
      </p>
    </div>

    <pre
      v-else
      class="min-h-0 flex-1 overflow-auto px-3 py-2 font-mono text-[11px] leading-relaxed break-all whitespace-pre-wrap select-text"
      >{{ yaml }}</pre
    >
  </aside>
</template>
