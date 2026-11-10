<script setup lang="ts">
/**
 * The Kubernetes half of the session editor.
 *
 * Two things it deliberately does. It never shows a kubeconfig back: the document goes in as a
 * secret and only contexts come out, so an edited session says one is stored instead of pretending
 * the field is empty for lack of one. And it reads contexts by asking the shell — the file is
 * parsed there, and what crosses back is names, never the document.
 *
 * Reading contexts during the parallel build answers with an unsupported-command envelope. That is
 * shown where the button is, because it is an answer about this form and not a reason to lose it.
 */
import { LoaderCircle, RefreshCw } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import type { KubeconfigSource } from "@/types/k8s";

const props = defineProps<{ submitted: boolean }>();

const { t } = useI18n();
const editor = useSessionEditorStore();

const sources: readonly { value: KubeconfigSource; labelKey: string }[] = [
  { value: "defaultPath", labelKey: "sessions.editor.kubernetes.sourceDefault" },
  { value: "imported", labelKey: "sessions.editor.kubernetes.sourceImported" },
];

/**
 * Every context that can be chosen, with the empty one first.
 *
 * Empty means "the kubeconfig's own current-context", which is a real answer and not an absence —
 * so it is a named option rather than the placeholder.
 */
const contextOptions = computed<SelectOption[]>(() => {
  const listed: SelectOption[] = [
    { value: "", label: t("sessions.editor.kubernetes.contextDefault") },
    ...editor.kubeContexts.map((context) => ({
      value: context.name,
      label: context.name,
      detail: [context.cluster, context.user, context.current ? t("sessions.editor.kubernetes.contextCurrent") : ""]
        .filter((part) => part !== "")
        .join(" · "),
    })),
  ];
  const stored = editor.kubeContext;
  if (stored !== "" && !editor.kubeContexts.some((context) => context.name === stored)) {
    // A context saved earlier stays choosable before this dialog has read anything: the saved name
    // is the only trace of it here, and hiding it would make the field look unanswered.
    listed.push({ value: stored, label: stored });
  }
  return listed;
});

const viaSshOptions = computed<SelectOption[]>(() => [
  { value: "", label: t("sessions.editor.kubernetes.viaSshNone") },
  ...editor.sshSessions.map((session) => ({
    value: session.id,
    label: session.name,
    detail: `${session.user}@${session.host}:${session.port}`,
  })),
]);

/**
 * An imported kubeconfig that is already in the vault cannot be read back, so listing its contexts
 * would mean reading whatever this machine's default file says instead. The button waits for a
 * paste; the hint below it says so.
 */
const needsPaste = computed(() => editor.kubeSource === "imported" && editor.importedKubeconfig.trim() === "");

const canReadContexts = computed(() => !editor.kubeContextsBusy && !needsPaste.value);

const contextsProblem = computed(() =>
  editor.kubeContextsFailure ? describeShellError(translate, editor.kubeContextsFailure) : null,
);

const kubeconfigProblem = computed(() =>
  props.submitted && editor.sessionProblems.kubeconfig ? t("sessions.editor.validation.kubeconfig") : null,
);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <section>
    <h3 class="text-xs font-medium">{{ t("sessions.editor.kubernetes.title") }}</h3>

    <div class="mt-2">
      <span class="block text-xs font-medium">{{ t("sessions.editor.kubernetes.source") }}</span>
      <div role="radiogroup" :aria-label="t('sessions.editor.kubernetes.source')" class="mt-1 flex flex-wrap gap-2">
        <button
          v-for="option in sources"
          :key="option.value"
          type="button"
          role="radio"
          :aria-checked="editor.kubeSource === option.value"
          class="flex h-8 items-center rounded-md border px-3 text-xs transition-colors"
          :class="
            editor.kubeSource === option.value
              ? 'border-primary/40 bg-primary/10 text-primary ring-1 ring-primary/30'
              : 'border-border text-foreground hover:bg-accent'
          "
          @click="editor.kubeSource = option.value"
        >
          {{ t(option.labelKey) }}
        </button>
      </div>
      <p class="mt-1 text-[11px] text-muted-foreground">
        {{
          editor.kubeSource === "imported"
            ? t("sessions.editor.kubernetes.sourceImportedHint")
            : t("sessions.editor.kubernetes.sourceDefaultHint")
        }}
      </p>
    </div>

    <div v-if="editor.kubeSource === 'imported'" class="mt-3">
      <label class="block text-xs font-medium" for="session-kubeconfig">{{
        t("sessions.editor.kubernetes.kubeconfig")
      }}</label>
      <textarea
        id="session-kubeconfig"
        v-model="editor.importedKubeconfig"
        rows="6"
        spellcheck="false"
        class="mt-1 w-full resize-y rounded-md border bg-background px-2.5 py-2 font-mono text-xs focus:outline-none"
        :class="kubeconfigProblem ? 'border-destructive' : 'border-border'"
        :placeholder="t('sessions.editor.kubernetes.kubeconfigPlaceholder')"
      ></textarea>
      <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.kubernetes.kubeconfigHint") }}</p>
      <p v-if="editor.kubeconfigStored" class="mt-1 text-[11px] text-muted-foreground">
        {{ t("sessions.editor.kubernetes.kubeconfigStored") }}
      </p>
      <p v-if="kubeconfigProblem" class="mt-1 text-[11px] text-destructive">{{ kubeconfigProblem }}</p>
    </div>

    <div class="mt-3 flex items-center gap-2">
      <button
        type="button"
        class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
        :disabled="!canReadContexts"
        @click="editor.readKubeContexts()"
      >
        <LoaderCircle v-if="editor.kubeContextsBusy" :size="12" class="animate-spin" />
        <RefreshCw v-else :size="12" />
        {{
          editor.kubeContextsBusy
            ? t("sessions.editor.kubernetes.reading")
            : t("sessions.editor.kubernetes.readContexts")
        }}
      </button>
      <span v-if="needsPaste" class="text-[11px] text-muted-foreground">
        {{ t("sessions.editor.kubernetes.readNeedsPaste") }}
      </span>
    </div>

    <p v-if="contextsProblem" class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
      <span>{{ contextsProblem.message }}</span>
      <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ contextsProblem.code }}</span>
    </p>

    <div class="mt-3 grid gap-3 sm:grid-cols-2">
      <div>
        <label class="block text-xs font-medium" for="session-kube-context">{{
          t("sessions.editor.kubernetes.context")
        }}</label>
        <Select
          id="session-kube-context"
          v-model="editor.kubeContext"
          class="mt-1"
          :options="contextOptions"
          :placeholder="t('sessions.editor.kubernetes.contextPlaceholder')"
        />
      </div>
      <div>
        <label class="block text-xs font-medium" for="session-kube-namespace">{{
          t("sessions.editor.kubernetes.namespace")
        }}</label>
        <input
          id="session-kube-namespace"
          v-model="editor.kubeNamespace"
          type="text"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:outline-none"
          :placeholder="t('sessions.editor.kubernetes.namespacePlaceholder')"
        />
        <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.kubernetes.namespaceHint") }}</p>
      </div>
    </div>

    <div class="mt-3">
      <label class="block text-xs font-medium" for="session-kube-via-ssh">{{
        t("sessions.editor.kubernetes.viaSsh")
      }}</label>
      <Select
        id="session-kube-via-ssh"
        v-model="editor.kubeViaSsh"
        class="mt-1"
        :options="viaSshOptions"
        :placeholder="t('sessions.editor.kubernetes.viaSshNone')"
      />
      <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.kubernetes.viaSshHint") }}</p>
    </div>
  </section>
</template>
