<script setup lang="ts">
/**
 * The container half of the session editor.
 *
 * It reads and writes the editor store directly, like the dialog it is drawn inside, so the rules
 * about what a container session may save stay in one place. What this file decides is only where a
 * problem is said, because that is the part that needs the field beside it.
 *
 * The saved SSH session is a reference and not a copy: the engine on the far side is reached with
 * that session's credentials, route and host key, so this form never repeats any of them.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import type { ContainerTransport } from "@/types/container";

const props = defineProps<{ submitted: boolean }>();

const { t } = useI18n();
const editor = useSessionEditorStore();

/** Where the engine runs: on this machine, or on the far side of a saved SSH session. */
const transports: readonly { value: ContainerTransport; labelKey: string }[] = [
  { value: "local", labelKey: "sessions.editor.container.transportLocal" },
  { value: "ssh", labelKey: "sessions.editor.container.transportSsh" },
];

const sshSessionOptions = computed<SelectOption[]>(() => {
  const listed: SelectOption[] = editor.sshSessions.map((session) => ({
    value: session.id,
    label: session.name,
    detail: `${session.user}@${session.host}:${session.port}`,
  }));
  const stored = editor.containerSshSession;
  if (stored !== "" && !listed.some((option) => option.value === stored)) {
    // The session it names may have been deleted since. It stays visible, because the reference is
    // the thing that has to be changed and a hidden one could not be.
    listed.unshift({ value: stored, label: stored });
  }
  return listed;
});

const sshSessionProblem = computed(() =>
  props.submitted && editor.sessionProblems.containerSshSession
    ? t("sessions.editor.validation.containerSshSession")
    : null,
);
</script>

<template>
  <section>
    <h3 class="text-xs font-medium">{{ t("sessions.editor.container.title") }}</h3>

    <div v-if="editor.containerRuntime === 'nerdctl'" class="mt-2">
      <label class="block text-xs font-medium" for="session-container-namespace">{{
        t("sessions.editor.container.namespace")
      }}</label>
      <input
        id="session-container-namespace"
        v-model="editor.containerNamespace"
        type="text"
        class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:outline-none"
        :placeholder="t('sessions.editor.container.namespacePlaceholder')"
      />
      <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.container.namespaceHint") }}</p>
    </div>

    <div class="mt-3">
      <span class="block text-xs font-medium">{{ t("sessions.editor.container.transport") }}</span>
      <div role="radiogroup" :aria-label="t('sessions.editor.container.transport')" class="mt-1 flex flex-wrap gap-2">
        <button
          v-for="option in transports"
          :key="option.value"
          type="button"
          role="radio"
          :aria-checked="editor.containerTransport === option.value"
          class="flex h-8 items-center rounded-md border px-3 text-xs transition-colors"
          :class="
            editor.containerTransport === option.value
              ? 'border-primary/40 bg-primary/10 text-primary ring-1 ring-primary/30'
              : 'border-border text-foreground hover:bg-accent'
          "
          @click="editor.containerTransport = option.value"
        >
          {{ t(option.labelKey) }}
        </button>
      </div>
    </div>

    <div v-if="editor.containerTransport === 'ssh'" class="mt-3">
      <label class="block text-xs font-medium" for="session-container-ssh">{{
        t("sessions.editor.container.sshSession")
      }}</label>
      <Select
        id="session-container-ssh"
        v-model="editor.containerSshSession"
        class="mt-1"
        :options="sshSessionOptions"
        :placeholder="t('sessions.editor.container.sshPick')"
      />
      <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.container.sshHint") }}</p>
      <p v-if="editor.sshSessions.length === 0" class="mt-1 text-[11px] text-muted-foreground">
        {{ t("sessions.editor.container.noSshSessions") }}
      </p>
      <p v-if="sshSessionProblem" class="mt-1 text-[11px] text-destructive">{{ sshSessionProblem }}</p>
    </div>
  </section>
</template>
