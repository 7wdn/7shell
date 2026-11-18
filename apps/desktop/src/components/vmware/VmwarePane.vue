<script setup lang="ts">
/**
 * The workspace for one virtual machine.
 *
 * Nothing is connected here: a machine is either running or not, and every question about it is
 * asked of `vmrun` when the tab is drawn. The two buttons that reach the guest do not open an
 * ad-hoc connection either — each one keeps an ordinary saved session up to date and opens it, so
 * credentials, keys and the host-key decision stay where every other session keeps them.
 *
 * The identity held by the session is a token a scan answered with, never a path: this pane edits
 * an account and two ports, and the host resolves everything else.
 */
import { FilePlus, FolderPlus, LoaderCircle, MonitorCog, Pause, Play, RefreshCw, Square, X } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import VmwareCredentialDialog from "@/components/vmware/VmwareCredentialDialog.vue";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import { isDesktopRuntime } from "@/lib/runtime";
import { vmGuestSession } from "@/lib/vmware/derive";
import { guestOsLabel } from "@/lib/vmware/guest";
import { useAppStore } from "@/stores/app";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore, type VmwareTab } from "@/stores/terminals";
import type { Session } from "@/types/session";
import type { HostLocationKind, HostLocationView, VmPowerAction, VmStatus } from "@/types/vmware";

const props = defineProps<{ tab: VmwareTab; active: boolean }>();

const { t } = useI18n();
const { toast } = useToast();
const app = useAppStore();
const library = useSessionsStore();
const terminals = useTerminalsStore();

/** The session as the library has it, which is newer than the tab's own snapshot after an edit. */
const session = computed(
  () => library.sessions.find((entry) => entry.id === props.tab.session.id) ?? props.tab.session,
);
const vmId = computed(() => session.value.vmware?.vmId ?? "");

const status = ref<VmStatus | null>(null);
const failure = ref<ShellError | null>(null);
const busy = ref<string | null>(null);
const confirmingStop = ref(false);
const confirmingOwnership = ref(false);

/** The SSH session waiting for a password: nothing is saved until one is typed. */
const pendingGuest = ref<{ guest: Session; vm: Session } | null>(null);
const passwordOpen = ref(false);
const passwordBusy = ref(false);

const locations = ref<readonly HostLocationView[]>([]);
const vmrestUser = ref("");
const vmrestPassword = ref("");
const sshUser = ref("");
const sshPort = ref("22");
const rdpUser = ref("");
const rdpPort = ref("3389");

const isWindows = computed(() => app.info?.os === "windows");
const isDesktop = computed(() => isDesktopRuntime());
const stateKey = computed(() =>
  status.value === null
    ? "checking"
    : status.value.running
      ? "running"
      : status.value.suspended
        ? "suspended"
        : "stopped",
);
const toolsKey = computed(() => {
  switch (status.value?.toolsState) {
    case "running":
    case "installed":
    case "notInstalled":
    case "notRunning":
      return status.value.toolsState;
    default:
      return "unknown";
  }
});

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function failureText(error: ShellError): string {
  return describeShellError(translate, error).message;
}

/** What VMware itself said, when it said anything: the sentence above is this build's wording. */
const failureDetail = computed(() =>
  failure.value ? (describeShellError(translate, failure.value).detail ?? "") : "",
);

watch(
  session,
  (current) => {
    sshUser.value = current.vmware?.sshUser ?? "";
    sshPort.value = String(current.vmware?.sshPort ?? 22);
    rdpUser.value = current.vmware?.rdpUser ?? "";
    rdpPort.value = String(current.vmware?.rdpPort ?? 3389);
  },
  { immediate: true },
);

/** Whether the machine is stopped with a lock that would refuse the next start. */
const staleLock = computed(
  () => status.value !== null && !status.value.running && !status.value.suspended && status.value.locked,
);
const credentialTarget = computed(() => {
  const target = pendingGuest.value;
  return target ? `${target.guest.user}@${target.guest.host}` : "";
});

async function refresh(): Promise<VmStatus | null> {
  if (vmId.value === "") {
    return null;
  }
  busy.value = "refresh";
  failure.value = null;
  try {
    status.value = await backend.vmware.status(vmId.value);
    return status.value;
  } catch (error) {
    failure.value = asShellError(error, "vmware_status");
    return null;
  } finally {
    busy.value = null;
  }
}

/**
 * Refreshes the readings after a failed action without wiping the failure that explains it.
 *
 * A start that hit a leftover lock has to leave both on screen: the error, and the lock the pane
 * is about to offer to take ownership of.
 */
async function reloadStatus(): Promise<void> {
  if (vmId.value === "") {
    return;
  }
  try {
    status.value = await backend.vmware.status(vmId.value);
  } catch {
    // The action's own failure is the one worth showing.
  }
}

/** Writes the account and port fields back onto the VM session before anything uses them. */
async function persistProfile(): Promise<Session> {
  const current = session.value;
  const settings = current.vmware;
  if (!settings) {
    return current;
  }
  return library.save({
    ...current,
    vmware: {
      ...settings,
      sshUser: sshUser.value.trim() === "" ? undefined : sshUser.value.trim(),
      sshPort: Number(sshPort.value) || 22,
      rdpUser: rdpUser.value.trim() === "" ? undefined : rdpUser.value.trim(),
      rdpPort: Number(rdpPort.value) || 3389,
    },
    updatedAt: Math.floor(Date.now() / 1000),
  });
}

async function act(action: VmPowerAction, acknowledged: boolean): Promise<void> {
  busy.value = action;
  failure.value = null;
  try {
    await backend.vmware.power(vmId.value, action, acknowledged);
    await refresh();
  } catch (error) {
    failure.value = asShellError(error, "vmware_power");
    await reloadStatus();
  } finally {
    busy.value = null;
  }
}

async function stop(): Promise<void> {
  confirmingStop.value = false;
  await act("stop", true);
}

/** Starts the machine, first offering to take ownership when a leftover lock would refuse it. */
function requestStart(): void {
  if (staleLock.value) {
    confirmingOwnership.value = true;
    return;
  }
  void act("start", false);
}

async function takeOwnershipAndStart(): Promise<void> {
  confirmingOwnership.value = false;
  busy.value = "ownership";
  failure.value = null;
  let taken = false;
  try {
    await backend.vmware.takeOwnership(vmId.value, true);
    taken = true;
  } catch (error) {
    failure.value = asShellError(error, "vmware_take_ownership");
    await reloadStatus();
  } finally {
    busy.value = null;
  }
  if (taken) {
    await act("start", false);
  }
}

async function openGuest(kind: "ssh" | "rdp"): Promise<void> {
  busy.value = kind;
  failure.value = null;
  try {
    const current = await persistProfile();
    const fresh = await backend.vmware.status(vmId.value);
    status.value = fresh;
    if (!fresh.running) {
      toast(t("sessions.vmware.connect.powerOnFirst"), "refusal");
      return;
    }
    if (!fresh.ip) {
      toast(t("sessions.vmware.connect.noAddress"), "refusal");
      return;
    }
    if (kind === "ssh" && (current.vmware?.sshUser ?? "").trim() === "") {
      toast(t("sessions.vmware.connect.userRequired"), "refusal");
      return;
    }

    const reference = kind === "ssh" ? current.vmware?.sshSession : current.vmware?.rdpSession;
    const existing = reference ? library.sessions.find((entry) => entry.id === reference) : undefined;
    const guest = vmGuestSession(current, kind, fresh.ip, existing);

    // An SSH session with a password method and no stored password cannot connect. Ask for one
    // before anything is written: a cancelled question leaves no half-made session behind.
    if (kind === "ssh" && guest.auth.method === "password" && !guest.auth.hasSecret) {
      pendingGuest.value = { guest, vm: current };
      passwordOpen.value = true;
      return;
    }
    await finishGuest(current, guest);
  } catch (error) {
    failure.value = asShellError(error, "vmware_guest_open");
  } finally {
    busy.value = null;
  }
}

/** Saves the session, links it to the VM, and opens it. */
async function finishGuest(current: Session, guest: Session): Promise<void> {
  const reference = guest.protocol === "ssh" ? current.vmware?.sshSession : current.vmware?.rdpSession;
  const saved = await library.save(guest);
  if (saved.id !== reference) {
    const settings = current.vmware;
    if (settings) {
      await library.save({
        ...current,
        vmware: {
          ...settings,
          ...(guest.protocol === "ssh" ? { sshSession: saved.id } : { rdpSession: saved.id }),
        },
      });
    }
  }
  await terminals.open(saved);
}

/** The password answered the question: store it with the session and continue where it stopped. */
async function connectWithPassword(password: string): Promise<void> {
  const pending = pendingGuest.value;
  if (!pending) {
    return;
  }
  passwordBusy.value = true;
  failure.value = null;
  try {
    // One password answers both questions: the SSH session that was just made, and the guest
    // account the machine's session keeps for guest operations the agent may ask for later.
    const vm = await library.save(pending.vm, { value: password });
    const saved = await library.save(pending.guest, { value: password });
    pendingGuest.value = null;
    passwordOpen.value = false;
    await finishGuest(vm, saved);
  } catch (error) {
    failure.value = asShellError(error, "vmware_guest_open");
  } finally {
    passwordBusy.value = false;
  }
}

/** A cancelled question leaves nothing half-made: the pending session is dropped with it. */
function closePassword(open: boolean): void {
  passwordOpen.value = open;
  if (!open) {
    pendingGuest.value = null;
  }
}

async function loadLocations(): Promise<void> {
  if (!isDesktop.value) {
    return;
  }
  try {
    locations.value = await backend.hostLocations.list();
  } catch {
    // The list is only needed by the operations that name a host place.
  }
}

async function addLocation(kind: HostLocationKind): Promise<void> {
  busy.value = "location";
  failure.value = null;
  try {
    const title = t(
      kind === "folder" ? "sessions.vmware.locations.chooseFolder" : "sessions.vmware.locations.chooseFile",
    );
    const chosen = await backend.hostLocations.choose(kind, title);
    if (chosen) {
      locations.value = [...locations.value.filter((entry) => entry.token !== chosen.token), chosen];
    }
  } catch (error) {
    failure.value = asShellError(error, "host_location_choose");
  } finally {
    busy.value = null;
  }
}

async function saveVmrest(): Promise<void> {
  try {
    await backend.vmrest.configSet(vmrestUser.value, vmrestPassword.value);
    await backend.vmrest.status();
    toast(t("sessions.vmware.vmrest.saved"), "receipt");
  } catch (error) {
    failure.value = asShellError(error, "vmrest_config_set");
  }
}

async function startVmrest(): Promise<void> {
  busy.value = "vmrest";
  failure.value = null;
  try {
    await backend.vmrest.start();
    toast(t("sessions.vmware.vmrest.started"), "receipt");
  } catch (error) {
    failure.value = asShellError(error, "vmrest_start");
  } finally {
    busy.value = null;
  }
}

async function forgetLocation(token: string): Promise<void> {
  try {
    await backend.hostLocations.forget(token);
    locations.value = locations.value.filter((entry) => entry.token !== token);
  } catch (error) {
    failure.value = asShellError(error, "host_location_forget");
  }
}

onMounted(() => {
  void refresh();
  void loadLocations();
});

watch(
  () => props.active,
  (active) => {
    if (active) {
      void refresh();
    }
  },
);
</script>

<template>
  <section class="relative flex min-h-0 flex-1 flex-col overflow-auto">
    <div class="mx-auto w-full max-w-2xl space-y-4 p-6">
      <header class="flex items-start justify-between gap-3">
        <div class="flex min-w-0 items-center gap-2">
          <MonitorCog :size="16" class="shrink-0 text-muted-foreground" />
          <div class="min-w-0">
            <h2 class="truncate text-sm font-medium">{{ session.name }}</h2>
            <p v-if="session.vmware?.vmId" class="truncate font-mono text-[10px] text-muted-foreground">
              {{ session.vmware.vmId }}
            </p>
          </div>
        </div>
        <button
          type="button"
          data-vmware-refresh
          class="grid size-7 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent disabled:opacity-60"
          :title="t('sessions.vmware.refresh')"
          :aria-label="t('sessions.vmware.refresh')"
          :disabled="busy !== null"
          @click="refresh()"
        >
          <LoaderCircle v-if="busy === 'refresh'" :size="13" class="animate-spin" />
          <RefreshCw v-else :size="13" />
        </button>
      </header>

      <div class="rounded-md border border-border p-4">
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-xs">
          <dt class="text-muted-foreground">{{ t("sessions.vmware.stateLabel") }}</dt>
          <dd class="flex items-center gap-2">
            <span
              class="size-1.5 rounded-full"
              :class="status?.running ? 'bg-success' : status?.suspended ? 'bg-warning' : 'bg-muted-foreground'"
            ></span>
            {{ t(`sessions.vmware.state.${stateKey}`) }}
          </dd>
          <dt class="text-muted-foreground">{{ t("sessions.vmware.systemLabel") }}</dt>
          <dd :title="status?.guestOs">
            {{ guestOsLabel(status?.guestOs) ?? status?.guestOs ?? "—" }}
          </dd>
          <dt class="text-muted-foreground">{{ t("sessions.vmware.toolsLabel") }}</dt>
          <dd class="text-muted-foreground">{{ t(`sessions.vmware.tools.${toolsKey}`) }}</dd>
          <dt class="text-muted-foreground">{{ t("sessions.vmware.addressLabel") }}</dt>
          <dd class="font-mono">{{ status?.ip ?? t("sessions.vmware.addressNone") }}</dd>
        </dl>

        <p
          v-if="staleLock"
          data-vmware-stale-lock
          class="mt-4 rounded-md border border-warning/40 bg-warning/5 px-3 py-2 text-[11px] leading-relaxed text-warning"
        >
          {{ t("sessions.vmware.lock.hint") }}
        </p>

        <div class="mt-4 flex flex-wrap gap-2">
          <button
            v-if="!status?.running"
            type="button"
            data-vmware-power-start
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-60"
            :disabled="busy !== null"
            @click="requestStart()"
          >
            <LoaderCircle v-if="busy === 'start' || busy === 'ownership'" :size="12" class="animate-spin" />
            <Play v-else :size="12" />
            {{ status?.suspended ? t("sessions.vmware.resume") : t("sessions.vmware.start") }}
          </button>
          <template v-else>
            <button
              type="button"
              data-vmware-power-stop
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="busy !== null"
              @click="confirmingStop = true"
            >
              <LoaderCircle v-if="busy === 'stop'" :size="12" class="animate-spin" />
              <Square v-else :size="12" />
              {{ t("sessions.vmware.stop") }}
            </button>
            <button
              type="button"
              data-vmware-power-suspend
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="busy !== null"
              @click="act('suspend', true)"
            >
              <LoaderCircle v-if="busy === 'suspend'" :size="12" class="animate-spin" />
              <Pause v-else :size="12" />
              {{ t("sessions.vmware.suspend") }}
            </button>
          </template>
        </div>
      </div>

      <div class="rounded-md border border-border p-4">
        <h3 class="text-xs font-medium">{{ t("sessions.vmware.sshTitle") }}</h3>
        <div class="mt-2 grid grid-cols-[1fr_90px_auto] items-end gap-2">
          <div class="min-w-0">
            <label class="block text-[11px] text-muted-foreground" for="vmware-ssh-user">{{
              t("sessions.vmware.user")
            }}</label>
            <input
              id="vmware-ssh-user"
              v-model="sshUser"
              type="text"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:outline-none"
              :placeholder="t('sessions.vmware.sshUserPlaceholder')"
            />
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="vmware-ssh-port">{{
              t("sessions.vmware.port")
            }}</label>
            <input
              id="vmware-ssh-port"
              v-model="sshPort"
              type="text"
              inputmode="numeric"
              class="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
            />
          </div>
          <button
            type="button"
            data-vmware-open-ssh
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-60"
            :disabled="busy !== null || status === null"
            @click="openGuest('ssh')"
          >
            <LoaderCircle v-if="busy === 'ssh'" :size="12" class="animate-spin" />
            {{ t("sessions.vmware.openSsh") }}
          </button>
        </div>

        <template v-if="isWindows">
          <h3 class="mt-4 text-xs font-medium">{{ t("sessions.vmware.rdpTitle") }}</h3>
          <div class="mt-2 grid grid-cols-[1fr_90px_auto] items-end gap-2">
            <div class="min-w-0">
              <label class="block text-[11px] text-muted-foreground" for="vmware-rdp-user">{{
                t("sessions.vmware.user")
              }}</label>
              <input
                id="vmware-rdp-user"
                v-model="rdpUser"
                type="text"
                autocapitalize="off"
                autocorrect="off"
                spellcheck="false"
                class="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:outline-none"
                :placeholder="t('sessions.vmware.rdpUserPlaceholder')"
              />
            </div>
            <div>
              <label class="block text-[11px] text-muted-foreground" for="vmware-rdp-port">{{
                t("sessions.vmware.port")
              }}</label>
              <input
                id="vmware-rdp-port"
                v-model="rdpPort"
                type="text"
                inputmode="numeric"
                class="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
              />
            </div>
            <button
              type="button"
              data-vmware-open-rdp
              class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="busy !== null || status === null"
              @click="openGuest('rdp')"
            >
              <LoaderCircle v-if="busy === 'rdp'" :size="12" class="animate-spin" />
              {{ t("sessions.vmware.openRdp") }}
            </button>
          </div>
        </template>

        <p class="mt-3 text-[11px] text-muted-foreground">{{ t("sessions.vmware.hint") }}</p>
      </div>

      <div v-if="isDesktop" class="rounded-md border border-border p-4" data-vmrest>
        <div class="flex items-center justify-between gap-2">
          <h3 class="text-xs font-medium">{{ t("sessions.vmware.vmrest.title") }}</h3>
          <button
            type="button"
            data-vmrest-start
            class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent disabled:opacity-60"
            :disabled="busy !== null"
            @click="startVmrest()"
          >
            <LoaderCircle v-if="busy === 'vmrest'" :size="12" class="animate-spin" />
            {{ t("sessions.vmware.vmrest.start") }}
          </button>
        </div>
        <div class="mt-2 grid grid-cols-[1fr_1fr_auto] items-end gap-2">
          <input
            v-model="vmrestUser"
            type="text"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            class="w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:outline-none"
            :placeholder="t('sessions.vmware.vmrest.user')"
          />
          <input
            v-model="vmrestPassword"
            type="password"
            autocomplete="new-password"
            class="w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs focus:outline-none"
            :placeholder="t('sessions.vmware.vmrest.password')"
          />
          <button
            type="button"
            data-vmrest-save
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
            @click="saveVmrest()"
          >
            {{ t("sessions.vmware.vmrest.save") }}
          </button>
        </div>
      </div>

      <div v-if="isDesktop" class="rounded-md border border-border p-4" data-vmware-locations>
        <div class="flex items-center justify-between gap-2">
          <h3 class="text-xs font-medium">{{ t("sessions.vmware.locations.title") }}</h3>
          <div class="flex gap-1">
            <button
              type="button"
              data-vmware-add-folder
              class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="busy !== null"
              @click="addLocation('folder')"
            >
              <FolderPlus :size="12" />
              {{ t("sessions.vmware.locations.folder") }}
            </button>
            <button
              type="button"
              data-vmware-add-file
              class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent disabled:opacity-60"
              :disabled="busy !== null"
              @click="addLocation('file')"
            >
              <FilePlus :size="12" />
              {{ t("sessions.vmware.locations.file") }}
            </button>
          </div>
        </div>
        <ul v-if="locations.length > 0" class="mt-2 space-y-1">
          <li
            v-for="location in locations"
            :key="location.token"
            class="flex items-center gap-2 text-xs"
            :data-vmware-location="location.token"
          >
            <span class="min-w-0 flex-1 truncate" :title="location.name">{{ location.name }}</span>
            <span class="shrink-0 text-[10px] text-muted-foreground">{{
              t(`sessions.vmware.locations.${location.kind}`)
            }}</span>
            <button
              type="button"
              class="shrink-0 text-muted-foreground transition-colors hover:text-destructive"
              :title="t('sessions.vmware.locations.forget')"
              :aria-label="t('sessions.vmware.locations.forget')"
              @click="forgetLocation(location.token)"
            >
              <X :size="12" />
            </button>
          </li>
        </ul>
      </div>

      <p
        v-if="failure"
        class="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
      >
        {{ failureText(failure) }}
        <span v-if="failureDetail" class="mt-1 block font-mono text-[10px] break-all opacity-80">{{
          failureDetail
        }}</span>
      </p>
    </div>

    <ConfirmDialog
      :open="confirmingStop"
      :title="t('sessions.vmware.stopTitle', { name: session.name })"
      :message="t('sessions.vmware.stopMessage')"
      :confirm-label="t('sessions.vmware.stop')"
      danger
      @confirm="stop()"
      @update:open="confirmingStop = false"
    />

    <!--
      A machine that is not running but still holds its lock will refuse every start. Taking
      ownership is what Workstation's own window offers; the question says what the lock is for
      and when removing it is wrong.
    -->
    <ConfirmDialog
      :open="confirmingOwnership"
      :title="t('sessions.vmware.lock.title', { name: session.name })"
      :message="t('sessions.vmware.lock.message')"
      :confirm-label="t('sessions.vmware.lock.confirm')"
      danger
      @confirm="takeOwnershipAndStart()"
      @update:open="confirmingOwnership = false"
    />

    <VmwareCredentialDialog
      :open="passwordOpen"
      :busy="passwordBusy"
      :target="credentialTarget"
      @confirm="connectWithPassword"
      @update:open="closePassword"
    />
  </section>
</template>
