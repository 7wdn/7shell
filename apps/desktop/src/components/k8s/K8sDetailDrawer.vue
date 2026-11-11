<script setup lang="ts">
/**
 * One object in full: its readings, and the document the server sent.
 *
 * The YAML tab can be edited and saved, which is a plain PUT of the whole document: the server's own
 * validation is the only answer that counts, so what it rejects is shown back here and kept in the
 * draft rather than guessed at beforehand. There is no diff and no dry run — the cluster has no
 * "would this be accepted" question that a general client can ask.
 *
 * A Secret is the one object whose values are not printed here, and for the same reason it is not
 * editable: the document on screen has `***` where the material is, and saving it back would write
 * the masks over the secret. Its keys are still listed, because which credential an object holds is
 * the useful half; the material would travel no further than a screenshot, which is exactly how a
 * secret leaves a machine.
 */
import { Copy, LoaderCircle, X } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { parse } from "yaml";

import { age, createdAtOf, idOf, nameOf, namespaceOf, textAt, type K8sObject } from "@/components/k8s/objects";
import type { ResourceDescriptor } from "@/components/k8s/resources";
import { redactedForDisplay, toYaml } from "@/components/k8s/yaml";
import { describeShellError } from "@/i18n/errors";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { cn } from "@/lib/utils";
import { useK8sStore } from "@/stores/k8s";

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
const store = useK8sStore();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const tab = ref<"detail" | "yaml">("detail");

const editing = ref(false);
const draft = ref("");
const saving = ref(false);
/** The YAML did not survive parsing, or there was nowhere to send it. A sentence, not a code. */
const draftError = ref("");
/** What the parser said, kept apart so the sentence above can stay translated. */
const draftDetail = ref("");
/** What the API server answered. The draft stays on screen until it is accepted. */
const saveFailure = ref<ShellError | null>(null);

/**
 * Opening another object starts again on its readings, not on the tab the last one was left on.
 *
 * Keyed by identity rather than by the object: a watch event replaces the object on every change, and
 * a person typing into the YAML tab must not be thrown back to the readings because a status field
 * moved.
 */
watch(
  () => (props.object ? idOf(props.object) : ""),
  () => {
    tab.value = "detail";
    resetEditing();
  },
);

function resetEditing(): void {
  editing.value = false;
  draft.value = "";
  draftError.value = "";
  draftDetail.value = "";
  saveFailure.value = null;
}

const shown = computed(() => props.object);

const sections = computed(() => props.descriptor?.detail ?? []);

const yaml = computed(() => (props.object ? toYaml(redactedForDisplay(props.object)) : ""));

/** Whether the document on screen is the object's own, or a masked copy that must not be saved. */
const editable = computed(() => props.descriptor !== null && props.descriptor.redactData !== true);

const title = computed(() => {
  if (!props.object || !props.descriptor) {
    return "";
  }
  const kind = props.descriptor.kindLabel ?? t(props.descriptor.labelKey);
  return `${kind} / ${namespaceOf(props.object) || t("k8s.detail.cluster")} / ${nameOf(props.object)}`;
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

/** The draft starts as the document on screen, which is the object the server last sent. */
function startEditing(): void {
  draft.value = yaml.value;
  draftError.value = "";
  draftDetail.value = "";
  saveFailure.value = null;
  editing.value = true;
}

function cancelEditing(): void {
  if (saving.value) {
    return;
  }
  resetEditing();
}

/**
 * Parses the draft and sends it as the object's replacement.
 *
 * The document is parsed here rather than by the API server because the server takes JSON for a PUT,
 * and sending the text as YAML would make the content type a guess. A parse failure never reaches
 * the cluster; a rejection does not clear the draft, because rewriting a rejected document is the
 * whole point of having it still on screen.
 */
async function save(): Promise<void> {
  const object = props.object;
  const descriptor = props.descriptor;
  if (!object || !descriptor || saving.value) {
    return;
  }
  draftError.value = "";
  draftDetail.value = "";
  saveFailure.value = null;

  let body: string;
  try {
    const document: unknown = parse(draft.value);
    if (document === null || typeof document !== "object" || Array.isArray(document)) {
      draftError.value = t("k8s.yaml.notObject");
      return;
    }
    body = JSON.stringify(document);
  } catch (error) {
    draftError.value = t("k8s.yaml.invalid");
    draftDetail.value = error instanceof Error ? error.message : String(error);
    return;
  }

  const connId = store.connectionIdOfObject(object);
  if (connId === null) {
    draftError.value = t("k8s.yaml.noConnection");
    return;
  }
  saving.value = true;
  try {
    await store.replaceObject(connId, descriptor, object, body);
    resetEditing();
  } catch (error) {
    saveFailure.value = asShellError(error, "k8s_save");
  } finally {
    saving.value = false;
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
      <div v-if="tab === 'yaml'" class="ml-auto flex items-center gap-1">
        <button
          v-if="!editing"
          type="button"
          class="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="copyYaml()"
        >
          <Copy :size="11" />
          {{ copied ? t("k8s.yaml.copied") : t("k8s.yaml.copy") }}
        </button>
        <button
          v-if="!editing && editable"
          type="button"
          class="rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="startEditing()"
        >
          {{ t("k8s.yaml.edit") }}
        </button>
        <template v-if="editing">
          <button
            type="button"
            class="flex items-center gap-1 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="saving"
            @click="save()"
          >
            <LoaderCircle v-if="saving" :size="11" class="animate-spin" />
            {{ t("k8s.yaml.save") }}
          </button>
          <button
            type="button"
            class="rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="saving"
            @click="cancelEditing()"
          >
            {{ t("common.action.cancel") }}
          </button>
        </template>
      </div>
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

    <div v-else class="flex min-h-0 flex-1 flex-col">
      <textarea
        v-if="editing"
        v-model="draft"
        spellcheck="false"
        class="min-h-0 flex-1 resize-none bg-background px-3 py-2 font-mono text-[11px] leading-relaxed focus:outline-none"
        :aria-label="t('k8s.yaml.editor')"
      />
      <pre
        v-else
        class="min-h-0 flex-1 overflow-auto px-3 py-2 font-mono text-[11px] leading-relaxed break-all whitespace-pre-wrap select-text"
        >{{ yaml }}</pre
      >

      <div
        v-if="editing && draftError"
        role="alert"
        class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-3 py-2"
      >
        <span class="block text-xs">{{ draftError }}</span>
        <span v-if="draftDetail" class="mt-0.5 block font-mono text-[11px] break-all text-muted-foreground">
          {{ draftDetail }}
        </span>
      </div>

      <div
        v-if="editing && saveFailure"
        role="alert"
        class="shrink-0 border-t border-destructive/40 bg-destructive/10 px-3 py-2"
      >
        <span class="block text-xs">{{ describeShellError(translate, saveFailure).message }}</span>
        <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">
          {{ describeShellError(translate, saveFailure).code }}
        </span>
      </div>

      <p
        v-if="!editing && props.descriptor?.redactData"
        class="shrink-0 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground"
      >
        {{ t("k8s.yaml.redacted") }}
      </p>
    </div>
  </aside>
</template>
