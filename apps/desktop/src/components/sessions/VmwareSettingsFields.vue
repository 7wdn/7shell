<script setup lang="ts">
/**
 * The VMware half of the session editor.
 *
 * The machine is picked from a live scan rather than typed: the identity this session stores is a
 * token a scan answered with, and the interface never composes or holds a `.vmx` path. An imported
 * session whose machine is no longer listed keeps its token visible, because a reference that
 * cannot be seen is a reference nobody can repair.
 *
 * The two accounts and ports are what the workspace uses when it makes a session that reaches the
 * guest; no password is asked for here. Credentials belong to that saved session, not to the VM.
 */
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import type { VmInfo } from "@/types/vmware";

const props = defineProps<{ submitted: boolean }>();

const { t } = useI18n();
const editor = useSessionEditorStore();

const machines = ref<readonly VmInfo[]>([]);
const scanning = ref(false);
const failure = ref<ShellError | null>(null);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const machineOptions = computed<SelectOption[]>(() => {
  const listed: SelectOption[] = machines.value.map((machine) => ({
    value: machine.id,
    label: machine.name,
    detail: machine.running ? t("sessions.vmware.state.running") : t("sessions.vmware.state.stopped"),
  }));
  const stored = editor.vmwareId.trim();
  if (stored !== "" && !listed.some((option) => option.value === stored)) {
    listed.unshift({ value: stored, label: t("sessions.editor.vmware.unknownMachine") });
  }
  return listed;
});

const problem = computed(() =>
  props.submitted && editor.sessionProblems.vmwareId ? t("sessions.editor.validation.vmwareId") : null,
);

async function scan(): Promise<void> {
  if (scanning.value) {
    return;
  }
  scanning.value = true;
  failure.value = null;
  try {
    machines.value = await backend.vmware.scan();
  } catch (error) {
    failure.value = asShellError(error, "vmware_scan");
  } finally {
    scanning.value = false;
  }
}

onMounted(() => {
  if (editor.isVmware) {
    void scan();
  }
});

watch(
  () => editor.isVmware,
  (isVmware) => {
    if (isVmware) {
      void scan();
    }
  },
);
</script>

<template>
  <section>
    <h3 class="text-xs font-medium">{{ t("sessions.editor.vmware.title") }}</h3>

    <div class="mt-2">
      <label class="block text-xs font-medium" for="session-vmware-machine">{{
        t("sessions.editor.vmware.machine")
      }}</label>
      <Select
        id="session-vmware-machine"
        v-model="editor.vmwareId"
        class="mt-1"
        :options="machineOptions"
        :placeholder="scanning ? t('sessions.vmware.scanning') : t('sessions.editor.vmware.machinePick')"
      />
      <p v-if="failure" class="mt-1 text-[11px] text-destructive">{{ describeShellError(translate, failure) }}</p>
      <p v-else class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.vmware.machineHint") }}</p>
      <p v-if="problem" class="mt-1 text-[11px] text-destructive">{{ problem }}</p>
    </div>

    <div class="mt-3 grid grid-cols-2 gap-2">
      <div>
        <label class="block text-xs font-medium" for="session-vmware-ssh-user">{{
          t("sessions.editor.vmware.sshUser")
        }}</label>
        <input
          id="session-vmware-ssh-user"
          v-model="editor.vmwareSshUser"
          type="text"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
          :placeholder="t('sessions.editor.vmware.sshUserPlaceholder')"
        />
      </div>
      <div>
        <label class="block text-xs font-medium" for="session-vmware-ssh-port">{{
          t("sessions.editor.vmware.sshPort")
        }}</label>
        <input
          id="session-vmware-ssh-port"
          v-model="editor.vmwareSshPort"
          type="text"
          inputmode="numeric"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:outline-none"
        />
      </div>
      <div>
        <label class="block text-xs font-medium" for="session-vmware-rdp-user">{{
          t("sessions.editor.vmware.rdpUser")
        }}</label>
        <input
          id="session-vmware-rdp-user"
          v-model="editor.vmwareRdpUser"
          type="text"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
          :placeholder="t('sessions.editor.vmware.rdpUserPlaceholder')"
        />
      </div>
      <div>
        <label class="block text-xs font-medium" for="session-vmware-rdp-port">{{
          t("sessions.editor.vmware.rdpPort")
        }}</label>
        <input
          id="session-vmware-rdp-port"
          v-model="editor.vmwareRdpPort"
          type="text"
          inputmode="numeric"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:outline-none"
        />
      </div>
    </div>
    <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.vmware.hint") }}</p>
  </section>
</template>
