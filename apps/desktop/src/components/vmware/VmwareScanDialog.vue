<script setup lang="ts">
/**
 * What this machine's Workstation holds, offered for import.
 *
 * A scan is a question, so the dialog asks it every time it opens rather than caching a list that
 * may describe a machine somebody removed an hour ago. Machines already in the library are shown
 * as such and cannot be imported twice; the identity is the token the scan answered with, which
 * is also what makes "already there" a fact and not a name comparison.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { Check, LoaderCircle, MonitorCog, Plus, RefreshCw } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import { groupPath } from "@/lib/sessions/groups";
import { vmSessionFromScan } from "@/lib/vmware/derive";
import { guestOsLabel } from "@/lib/vmware/guest";
import { TABLE, rowClass } from "@/lib/table";
import { cn } from "@/lib/utils";
import { useSessionsStore } from "@/stores/sessions";
import type { VmInfo } from "@/types/vmware";

/** The group an import lands in when nobody picked one. */
const DEFAULT_GROUP = "VMware";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const { toast } = useToast();
const library = useSessionsStore();

const machines = ref<readonly VmInfo[]>([]);
const chosen = ref<ReadonlySet<string>>(new Set());
const loading = ref(false);
const importing = ref(false);
const failure = ref<ShellError | null>(null);

/** Where the imported sessions land, and the inline field that makes a new group. */
const chosenGroup = ref("");
const addingGroup = ref(false);
const groupName = ref("");
const groupBusy = ref(false);
const groupField = ref<HTMLInputElement | null>(null);

const groupOptions = computed(() =>
  [...library.groups]
    .map((group) => ({ value: group.id, label: groupPath(library.groups, group.id) }))
    .sort((left, right) => left.label.localeCompare(right.label)),
);

const importedIds = computed(
  () =>
    new Set(
      library.sessions
        .filter((session) => session.protocol === "vmware")
        .map((session) => session.vmware?.vmId ?? "")
        .filter((id) => id !== ""),
    ),
);

const selectable = computed(() => machines.value.filter((machine) => !importedIds.value.has(machine.id)));
const chosenCount = computed(() => selectable.value.filter((machine) => chosen.value.has(machine.id)).length);
const allChosen = computed(() => selectable.value.length > 0 && chosenCount.value === selectable.value.length);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function failureText(error: ShellError): string {
  return describeShellError(translate, error).message;
}

const failureDetail = computed(() =>
  failure.value ? (describeShellError(translate, failure.value).detail ?? "") : "",
);

async function scan(): Promise<void> {
  if (loading.value) {
    return;
  }
  loading.value = true;
  failure.value = null;
  try {
    machines.value = await backend.vmware.scan();
    chosen.value = new Set();
  } catch (error) {
    failure.value = asShellError(error, "vmware_scan");
    machines.value = [];
  } finally {
    loading.value = false;
  }
}

function toggle(id: string): void {
  const next = new Set(chosen.value);
  if (!next.delete(id)) {
    next.add(id);
  }
  chosen.value = next;
}

function toggleAll(): void {
  chosen.value = allChosen.value ? new Set() : new Set(selectable.value.map((machine) => machine.id));
}

/** The group named like the default, when one already exists at the top level. */
function defaultGroup(): string {
  const existing = library.groups.find(
    (group) => !group.parentId && group.name.trim().toLowerCase() === DEFAULT_GROUP.toLowerCase(),
  );
  return existing?.id ?? "";
}

function startGroupAdd(): void {
  addingGroup.value = true;
  groupName.value = "";
  void nextTick().then(() => groupField.value?.focus());
}

function closeGroupAdd(): void {
  addingGroup.value = false;
  groupName.value = "";
}

async function commitGroupAdd(): Promise<void> {
  const name = groupName.value.trim();
  if (name === "" || groupBusy.value) {
    closeGroupAdd();
    return;
  }
  groupBusy.value = true;
  try {
    const created = await library.addGroup(name, null);
    if (created) {
      chosenGroup.value = created.id;
      closeGroupAdd();
    } else {
      failure.value = library.failure;
    }
  } finally {
    groupBusy.value = false;
  }
}

/** The group to import into: the chosen one, the existing default, or a new default. */
async function targetGroup(): Promise<string | null> {
  if (chosenGroup.value !== "") {
    return chosenGroup.value;
  }
  const existing = defaultGroup();
  if (existing !== "") {
    return existing;
  }
  const created = await library.addGroup(DEFAULT_GROUP, null);
  if (!created) {
    failure.value = library.failure;
    return null;
  }
  chosenGroup.value = created.id;
  return created.id;
}

async function importSelected(): Promise<void> {
  if (importing.value || chosenCount.value === 0) {
    return;
  }
  importing.value = true;
  failure.value = null;
  let imported = 0;
  try {
    const groupId = await targetGroup();
    if (groupId === null) {
      return;
    }
    for (const machine of selectable.value) {
      if (!chosen.value.has(machine.id)) {
        continue;
      }
      await library.save(vmSessionFromScan(machine, groupId));
      imported += 1;
    }
    toast(t("sessions.vmwareScan.imported", { count: String(imported) }), "result");
    emit("update:open", false);
  } catch (error) {
    failure.value = asShellError(error, "vmware_import");
  } finally {
    importing.value = false;
  }
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      chosenGroup.value = defaultGroup();
      addingGroup.value = false;
      void scan();
    }
  },
);
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(36rem,calc(100vh-3rem))] w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        data-vmware-scan-dialog
      >
        <div class="flex items-start gap-2.5 px-4 pt-4">
          <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background">
            <MonitorCog :size="14" class="text-muted-foreground" />
          </span>
          <div class="min-w-0 flex-1">
            <DialogTitle class="text-sm font-medium">{{ t("sessions.vmwareScan.title") }}</DialogTitle>
            <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
              {{ t("sessions.vmwareScan.description") }}
            </DialogDescription>
          </div>
          <button
            type="button"
            class="grid size-7 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent disabled:opacity-60"
            :title="t('sessions.vmwareScan.rescan')"
            :aria-label="t('sessions.vmwareScan.rescan')"
            :disabled="loading || importing"
            @click="scan()"
          >
            <LoaderCircle v-if="loading" :size="13" class="animate-spin" />
            <RefreshCw v-else :size="13" />
          </button>
        </div>

        <div class="mx-4 mt-3 min-h-0 flex-1 overflow-auto rounded-md border border-border">
          <div
            v-if="loading && machines.length === 0"
            class="flex items-center justify-center gap-2 p-6 text-xs text-muted-foreground"
          >
            <LoaderCircle :size="13" class="animate-spin" />
            {{ t("sessions.vmwareScan.scanning") }}
          </div>
          <p v-else-if="failure" class="p-4 text-xs text-destructive" role="alert">
            {{ failureText(failure) }}
            <span v-if="failureDetail" class="mt-1 block font-mono text-[10px] break-all opacity-80">{{
              failureDetail
            }}</span>
          </p>
          <p v-else-if="machines.length === 0" class="p-6 text-center text-xs text-muted-foreground">
            {{ t("sessions.vmwareScan.empty") }}
          </p>
          <table v-else :aria-label="t('sessions.vmwareScan.title')" :class="cn(TABLE.page.table, 'text-left')">
            <thead :class="TABLE.page.head">
              <tr :class="TABLE.page.headRow">
                <th :class="TABLE.page.headCell" class="w-8 pl-3">
                  <input
                    type="checkbox"
                    :checked="allChosen"
                    :disabled="selectable.length === 0 || importing"
                    :aria-label="t('sessions.vmwareScan.selectAll')"
                    @change="toggleAll()"
                  />
                </th>
                <th :class="TABLE.page.headCell">{{ t("sessions.vmwareScan.machine") }}</th>
                <th :class="TABLE.page.headCell">{{ t("sessions.vmwareScan.guest") }}</th>
                <th :class="TABLE.page.headCell" class="w-32">{{ t("sessions.vmwareScan.address") }}</th>
                <th :class="TABLE.page.headCell" class="w-24">{{ t("sessions.vmwareScan.state") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="machine in machines"
                :key="machine.id"
                :class="rowClass('page')"
                :data-vmware-scan-row="machine.id"
              >
                <td :class="TABLE.page.cell" class="pl-3 align-middle">
                  <input
                    type="checkbox"
                    :checked="chosen.has(machine.id)"
                    :disabled="importedIds.has(machine.id) || importing"
                    :aria-label="machine.name"
                    @change="toggle(machine.id)"
                  />
                </td>
                <td :class="TABLE.page.cell" class="align-middle">
                  <span class="block truncate" :class="importedIds.has(machine.id) ? 'opacity-60' : ''">{{
                    machine.name
                  }}</span>
                  <span v-if="importedIds.has(machine.id)" class="block text-[10px] text-muted-foreground">
                    {{ t("sessions.vmwareScan.alreadyImported") }}
                  </span>
                </td>
                <td
                  :class="TABLE.page.cell"
                  class="align-middle font-mono text-[10px] text-muted-foreground"
                  :title="machine.guestOs"
                >
                  {{ guestOsLabel(machine.guestOs) ?? machine.guestOs ?? "—" }}
                </td>
                <td :class="TABLE.page.cell" class="align-middle font-mono text-[10px] text-muted-foreground">
                  {{
                    machine.running
                      ? (machine.ip ?? t("sessions.vmwareScan.addressPending"))
                      : t("sessions.vmwareScan.addressOff")
                  }}
                </td>
                <td :class="TABLE.page.cell" class="align-middle">
                  {{ machine.running ? t("sessions.vmwareScan.running") : t("sessions.vmwareScan.stopped") }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <div class="flex items-center justify-between gap-2 px-4 py-3">
          <div class="flex min-w-0 items-center gap-1.5">
            <label
              class="shrink-0 text-[11px] text-muted-foreground"
              :for="addingGroup ? 'vmware-group-name' : 'vmware-import-group'"
            >
              {{ t("sessions.vmwareScan.group") }}
            </label>
            <template v-if="!addingGroup">
              <Select
                id="vmware-import-group"
                v-model="chosenGroup"
                class="w-40"
                :options="groupOptions"
                :placeholder="t('sessions.vmwareScan.groupDefault')"
              />
              <button
                type="button"
                data-vmware-group-add
                class="grid size-7 shrink-0 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent disabled:opacity-60"
                :title="t('sessions.vmwareScan.groupAdd')"
                :aria-label="t('sessions.vmwareScan.groupAdd')"
                :disabled="groupBusy || importing"
                @click="startGroupAdd()"
              >
                <Plus :size="12" />
              </button>
            </template>
            <template v-else>
              <input
                id="vmware-group-name"
                ref="groupField"
                v-model="groupName"
                type="text"
                autocomplete="off"
                spellcheck="false"
                class="h-7 w-40 rounded-md border border-border bg-background px-2 text-xs focus:ring-1 focus:ring-ring focus:outline-none"
                :placeholder="t('sessions.vmwareScan.groupName')"
                @keydown.enter.prevent="commitGroupAdd()"
                @keydown.esc.prevent="closeGroupAdd()"
              />
              <button
                type="button"
                data-vmware-group-create
                class="grid size-7 shrink-0 place-items-center rounded-md bg-primary text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-60"
                :title="t('sessions.vmwareScan.groupCreate')"
                :aria-label="t('sessions.vmwareScan.groupCreate')"
                :disabled="groupBusy"
                @click="commitGroupAdd()"
              >
                <LoaderCircle v-if="groupBusy" :size="12" class="animate-spin" />
                <Check v-else :size="12" />
              </button>
            </template>
          </div>
          <div class="flex shrink-0 gap-2">
            <button
              type="button"
              class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
              @click="emit('update:open', false)"
            >
              {{ t("common.action.cancel") }}
            </button>
            <button
              type="button"
              data-vmware-scan-import
              class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
              :disabled="importing || chosenCount === 0"
              @click="importSelected()"
            >
              <LoaderCircle v-if="importing" :size="12" class="animate-spin" />
              {{
                chosenCount > 0
                  ? t("sessions.vmwareScan.importCount", { count: String(chosenCount) })
                  : t("sessions.vmwareScan.import")
              }}
            </button>
          </div>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
