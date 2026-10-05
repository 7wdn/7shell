<script setup lang="ts">
/**
 * The session editor.
 *
 * Two things this dialog deliberately does not hold. It never holds a stored password: the wire
 * types have no field for one, so an existing secret is described in a sentence and left alone
 * unless something new is typed — and only then is a `secret` sent with the save. And it never
 * holds a local path: the key file is chosen by a native dialog inside Rust, which answers with a
 * display name, an algorithm and a fingerprint, none of which reveal where the file lives.
 *
 * The authentication methods come from the descriptor table, so this dialog cannot drift from what
 * the backend actually implements. The two the build does not implement are shown disabled rather
 * than hidden: the difference between "missing here" and "not offered" matters when a session is
 * imported from somewhere else later.
 *
 * The draft itself — including the route and the forwards — lives in `stores/sessionEditor`, which
 * owns the rules those have to satisfy. This component binds the fields and turns the store's
 * problem codes back into sentences, because that is the only place a translation exists.
 */
import { ArrowRight, ChevronDown, ChevronUp, KeyRound, LoaderCircle, Plus, Trash2, X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import Select, { type SelectOption } from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import {
  useSessionEditorStore,
  type ForwardDraft,
  type ForwardProblem,
  type SessionFieldProblem,
} from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import type { Session, ForwardMode } from "@/types/session";
import { AUTH_METHODS, type AuthMethodId } from "@/types/generated/protocols";

const props = defineProps<{ open: boolean; session: Session | null }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const library = useSessionsStore();
const editor = useSessionEditorStore();

/**
 * Methods the transport refuses.
 *
 * The agent is no longer one of them: the machine's agent is asked to sign, and the key never
 * reaches this build. Pageant still is — it speaks a protocol of its own — and it says so in the
 * editor rather than failing at the first connection.
 */
const UNSUPPORTED_METHODS: readonly AuthMethodId[] = ["pageant"];

/** The two kinds of forward this build can bring up. */
const forwardModes = computed<SelectOption[]>(() => [
  { value: "local", label: t("sessions.editor.forwards.modeLocal") },
  { value: "dynamic", label: t("sessions.editor.forwards.modeDynamic") },
]);

/** The hop the picker is pointing at, until it is added and the picker goes back to its prompt. */
const hopToAdd = ref("");

/** Every group there is, with "no group" first — the answer a session starts out with. */
const groupOptions = computed<SelectOption[]>(() => [
  { value: "", label: t("sessions.editor.groupNone") },
  ...library.groups.map((group) => ({ value: group.id, label: group.name })),
]);

/** The same list the descriptors carry, with the two this build does not implement unchoosable. */
const methodOptions = computed<SelectOption[]>(() =>
  AUTH_METHODS.map((option) => ({
    value: option.id,
    label: t(option.labelKey),
    disabled: UNSUPPORTED_METHODS.includes(option.id),
  })),
);

/** The sessions a hop can be: each one by name, with the address it would be reached at under it. */
const hopOptions = computed<SelectOption[]>(() =>
  editor.hopChoices.map((session) => ({
    value: session.id,
    label: session.name,
    detail: hopEndpoint(session),
  })),
);

const submitted = ref(false);
const busy = ref(false);
const keyBusy = ref(false);
const problem = ref<ShellError | null>(null);

const title = computed(() => (props.session ? t("sessions.editor.titleEdit") : t("sessions.editor.titleNew")));
const storedSecret = computed(() => props.session?.auth.hasSecret ?? false);
const canAddHop = computed(() => hopToAdd.value !== "");

/** Every rule is reported next to the field it belongs to, which is why they are collected here. */
const errors = computed<Partial<Record<SessionFieldProblem, string>>>(() => {
  const found: Partial<Record<SessionFieldProblem, string>> = {};
  if (editor.sessionProblems.name) {
    found.name = t("sessions.editor.validation.name");
  }
  if (editor.sessionProblems.host) {
    found.host = t("sessions.editor.validation.host");
  }
  if (editor.sessionProblems.user) {
    found.user = t("sessions.editor.validation.user");
  }
  if (editor.sessionProblems.port) {
    found.port = t("sessions.editor.validation.port");
  }
  if (editor.sessionProblems.key) {
    found.key = t("sessions.editor.validation.key");
  }
  return found;
});

const problemText = computed(() => (problem.value ? describeShellError(translate, problem.value) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function hopEndpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

function forwardMessage(problem: ForwardProblem): string {
  switch (problem) {
    case "bindHost":
      return t("sessions.editor.forwards.validation.bindHost");
    case "bindPort":
      return t("sessions.editor.forwards.validation.bindPort");
    case "targetHost":
      return t("sessions.editor.forwards.validation.targetHost");
    case "targetPort":
      return t("sessions.editor.forwards.validation.targetPort");
  }
}

/** One line per rule a row breaks, so a row with two bad ports says so twice rather than once. */
function forwardMessages(row: ForwardDraft): string[] {
  return (editor.forwardProblems[row.id] ?? []).map(forwardMessage);
}

function forwardProblem(row: ForwardDraft, field: ForwardProblem): boolean {
  return (editor.forwardProblems[row.id] ?? []).includes(field);
}

watch(
  () => [props.open, props.session] as const,
  ([open]) => {
    if (!open) {
      return;
    }
    editor.fill(props.session);
    hopToAdd.value = "";
    submitted.value = false;
    busy.value = false;
    keyBusy.value = false;
    problem.value = null;
  },
  { immediate: true },
);

function addSelectedHop(): void {
  editor.addHop(hopToAdd.value);
  hopToAdd.value = "";
}

async function save(): Promise<void> {
  submitted.value = true;
  problem.value = null;
  if (editor.hasProblems) {
    return;
  }

  busy.value = true;
  try {
    await library.save(editor.draft(), editor.secretInput());
    emit("update:open", false);
  } catch (error) {
    problem.value = asShellError(error, "session_save");
  } finally {
    busy.value = false;
  }
}

/** The passphrase is passed in with the import because the key is parsed while it is read. */
async function chooseKey(): Promise<void> {
  keyBusy.value = true;
  problem.value = null;
  try {
    const info = await backend.keys.import(
      t("sessions.editor.key.dialogTitle"),
      editor.passphrase === "" ? undefined : editor.passphrase,
    );
    if (info) {
      editor.attachedKey = info;
    }
    editor.passphrase = "";
  } catch (error) {
    problem.value = asShellError(error, "key_import");
  } finally {
    keyBusy.value = false;
  }
}

/** Whether the question about forgetting the attached key is on screen. */
const forgetting = ref(false);

/** Asking to forget the key: the question comes first, because the material does not come back. */
function requestForget(): void {
  forgetting.value = true;
}

async function forgetKey(): Promise<void> {
  const info = editor.attachedKey;
  forgetting.value = false;
  problem.value = null;
  if (!info?.keyRef) {
    editor.attachedKey = null;
    return;
  }
  try {
    await backend.keys.forget(info.keyRef);
    editor.attachedKey = null;
  } catch (error) {
    problem.value = asShellError(error, "key_forget");
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ title }}</DialogTitle>
          <DialogDescription class="sr-only">{{ t("sessions.editor.description") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <form class="max-h-[70vh] space-y-4 overflow-auto px-4 py-4" @submit.prevent="save()">
          <div class="grid gap-4 sm:grid-cols-2">
            <div>
              <label class="block text-xs font-medium" for="session-name">{{ t("sessions.editor.name") }}</label>
              <input
                id="session-name"
                v-model="editor.name"
                type="text"
                class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
                :class="submitted && errors.name ? 'border-destructive' : 'border-border'"
                :placeholder="t('sessions.editor.namePlaceholder')"
              />
              <p v-if="submitted && errors.name" class="mt-1 text-[11px] text-destructive">{{ errors.name }}</p>
            </div>

            <div>
              <label class="block text-xs font-medium" for="session-group">{{ t("sessions.editor.group") }}</label>
              <Select id="session-group" v-model="editor.groupId" class="mt-1" :options="groupOptions" />
            </div>

            <div class="sm:col-span-2">
              <label class="block text-xs font-medium" for="session-host">{{ t("sessions.editor.host") }}</label>
              <input
                id="session-host"
                v-model="editor.host"
                type="text"
                class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
                :class="submitted && errors.host ? 'border-destructive' : 'border-border'"
                :placeholder="t('sessions.editor.hostPlaceholder')"
              />
              <p v-if="submitted && errors.host" class="mt-1 text-[11px] text-destructive">{{ errors.host }}</p>
            </div>

            <div>
              <label class="block text-xs font-medium" for="session-port">{{ t("sessions.editor.port") }}</label>
              <input
                id="session-port"
                v-model="editor.port"
                type="text"
                inputmode="numeric"
                class="mt-1 w-full rounded-md border bg-background px-3 py-2 font-mono text-sm focus:outline-none"
                :class="submitted && errors.port ? 'border-destructive' : 'border-border'"
              />
              <p v-if="submitted && errors.port" class="mt-1 text-[11px] text-destructive">{{ errors.port }}</p>
            </div>

            <div>
              <label class="block text-xs font-medium" for="session-user">{{ t("sessions.editor.user") }}</label>
              <input
                id="session-user"
                v-model="editor.user"
                type="text"
                class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
                :class="submitted && errors.user ? 'border-destructive' : 'border-border'"
                :placeholder="t('sessions.editor.userPlaceholder')"
              />
              <p v-if="submitted && errors.user" class="mt-1 text-[11px] text-destructive">{{ errors.user }}</p>
            </div>
          </div>

          <section>
            <label class="block text-xs font-medium" for="session-method">{{ t("sessions.editor.method") }}</label>
            <Select id="session-method" v-model="editor.method" class="mt-1" :options="methodOptions" />
            <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.methodUnsupported") }}</p>

            <div v-if="editor.method === 'password' || editor.method === 'keyboardInteractive'" class="mt-3">
              <label class="block text-xs font-medium" for="session-secret">{{
                t("sessions.editor.secret.label")
              }}</label>
              <input
                id="session-secret"
                v-model="editor.secret"
                type="password"
                autocomplete="off"
                class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
                :placeholder="
                  storedSecret ? t('sessions.editor.secret.storedPlaceholder') : t('sessions.editor.secret.placeholder')
                "
              />
              <p class="mt-1 text-[11px] text-muted-foreground">
                {{ storedSecret ? t("sessions.editor.secret.stored") : t("sessions.editor.secret.notStored") }}
              </p>
            </div>

            <div v-else-if="editor.method === 'publicKey'" class="mt-3 space-y-3">
              <div v-if="editor.attachedKey" class="rounded-md border border-border bg-background px-3 py-3">
                <p class="flex items-center gap-2 text-xs font-medium">
                  <KeyRound :size="13" class="text-muted-foreground" />
                  {{ t("sessions.editor.key.attached") }}
                </p>
                <dl class="mt-2 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[11px]">
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.name") }}</dt>
                  <dd class="font-mono break-all">{{ editor.attachedKey.displayName }}</dd>
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.algorithm") }}</dt>
                  <dd class="font-mono">{{ editor.attachedKey.algorithm }}</dd>
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.fingerprint") }}</dt>
                  <dd class="font-mono break-all">{{ editor.attachedKey.fingerprint }}</dd>
                </dl>
                <p class="mt-2 text-[11px] text-muted-foreground">
                  {{
                    editor.attachedKey.hasPassphrase
                      ? t("sessions.editor.key.passphraseStored")
                      : t("sessions.editor.key.noPassphraseStored")
                  }}
                </p>
                <button
                  type="button"
                  class="mt-2 flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
                  @click="requestForget()"
                >
                  <Trash2 :size="12" />
                  {{ t("sessions.editor.key.forget") }}
                </button>
              </div>
              <p v-else class="text-[11px] text-muted-foreground">{{ t("sessions.editor.key.none") }}</p>

              <label class="flex items-center gap-2 text-xs">
                <input v-model="editor.keyHasPassphrase" type="checkbox" class="size-3.5 rounded-sm border-border" />
                {{ t("sessions.editor.key.hasPassphrase") }}
              </label>

              <div v-if="editor.keyHasPassphrase">
                <input
                  v-model="editor.passphrase"
                  type="password"
                  autocomplete="off"
                  class="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
                  :placeholder="t('sessions.editor.key.passphrasePlaceholder')"
                />
                <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.key.passphraseHint") }}</p>
              </div>

              <button
                type="button"
                class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
                :disabled="keyBusy"
                @click="chooseKey()"
              >
                <LoaderCircle v-if="keyBusy" :size="12" class="animate-spin" />
                <KeyRound v-else :size="12" />
                {{ t("sessions.editor.key.choose") }}
              </button>
              <p v-if="submitted && errors.key" class="text-[11px] text-destructive">{{ errors.key }}</p>
            </div>
          </section>

          <section>
            <h3 class="text-xs font-medium">{{ t("sessions.editor.terminal.title") }}</h3>

            <label class="mt-2 flex items-start gap-2 text-xs">
              <input v-model="editor.terminal.forwardAgent" type="checkbox" class="mt-0.5 size-3.5 accent-primary" />
              <span>
                <span>{{ t("sessions.editor.terminal.forwardAgent") }}</span>
                <!--
                  Said plainly, next to the switch: this is the one setting here that lends
                  something to the far side, and what it lends is the ability to sign with the keys
                  in this machine's agent.
                -->
                <span class="mt-0.5 block text-[11px] text-muted-foreground">
                  {{ t("sessions.editor.terminal.forwardAgentNote") }}
                </span>
              </span>
            </label>
          </section>

          <section>
            <h3 class="text-xs font-medium">{{ t("sessions.editor.jumps.title") }}</h3>
            <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.jumps.note") }}</p>

            <p v-if="editor.hops.length === 0" class="mt-2 text-[11px] text-muted-foreground">
              {{ t("sessions.editor.jumps.empty") }}
            </p>

            <ul v-else class="mt-2 space-y-1">
              <li
                v-for="(hop, index) in editor.hops"
                :key="hop.id + ':' + index"
                class="flex items-center gap-2 rounded-md border border-border bg-background px-2 py-1.5"
              >
                <span
                  class="grid size-4 shrink-0 place-items-center rounded-sm bg-muted font-mono text-[10px] text-muted-foreground"
                >
                  {{ index + 1 }}
                </span>
                <span class="min-w-0 flex-1">
                  <template v-if="hop.session">
                    <span class="block truncate text-xs">{{ hop.session.name }}</span>
                    <span class="block truncate font-mono text-[10px] text-muted-foreground">{{
                      hopEndpoint(hop.session)
                    }}</span>
                  </template>
                  <template v-else>
                    <span class="block truncate text-xs text-warning">{{ t("sessions.editor.jumps.missing") }}</span>
                    <span
                      class="block truncate font-mono text-[10px] text-muted-foreground"
                      :title="t('sessions.editor.jumps.missingTip')"
                      >{{ hop.id }}</span
                    >
                  </template>
                </span>
                <span class="flex shrink-0 items-center gap-0.5">
                  <button
                    type="button"
                    class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                    :disabled="index === 0"
                    :title="t('sessions.editor.jumps.up')"
                    :aria-label="t('sessions.editor.jumps.up')"
                    @click="editor.moveHop(index, -1)"
                  >
                    <ChevronUp :size="12" />
                  </button>
                  <button
                    type="button"
                    class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                    :disabled="index === editor.hops.length - 1"
                    :title="t('sessions.editor.jumps.down')"
                    :aria-label="t('sessions.editor.jumps.down')"
                    @click="editor.moveHop(index, 1)"
                  >
                    <ChevronDown :size="12" />
                  </button>
                  <button
                    type="button"
                    class="grid size-5 place-items-center rounded-sm text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
                    :title="t('sessions.editor.jumps.remove')"
                    :aria-label="t('sessions.editor.jumps.remove')"
                    @click="editor.removeHopAt(index)"
                  >
                    <X :size="12" />
                  </button>
                </span>
              </li>
            </ul>

            <div class="mt-2 flex gap-1.5">
              <Select
                v-model="hopToAdd"
                class="min-w-0 flex-1"
                :options="hopOptions"
                :placeholder="t('sessions.editor.jumps.pick')"
                :disabled="editor.hopChoices.length === 0"
                :aria-label="t('sessions.editor.jumps.choose')"
              />
              <button
                type="button"
                class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
                :disabled="!canAddHop"
                @click="addSelectedHop()"
              >
                <Plus :size="12" />
                {{ t("sessions.editor.jumps.add") }}
              </button>
            </div>
          </section>

          <section>
            <div class="flex items-center justify-between">
              <h3 class="text-xs font-medium">{{ t("sessions.editor.forwards.title") }}</h3>
              <button
                type="button"
                class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
                @click="editor.addForward()"
              >
                <Plus :size="12" />
                {{ t("sessions.editor.forwards.add") }}
              </button>
            </div>
            <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.forwards.hint") }}</p>

            <p v-if="editor.forwards.length === 0" class="mt-2 text-[11px] text-muted-foreground">
              {{ t("sessions.editor.forwards.empty") }}
            </p>

            <ul v-else class="mt-2 space-y-2">
              <li v-for="row in editor.forwards" :key="row.id">
                <div class="flex items-center gap-1">
                  <!--
                    What kind of forward this is, first, because it decides what the rest of the row
                    asks for: a local forward has a destination, a dynamic one is told where to go by
                    whoever connects and has none to type.
                  -->
                  <Select
                    :model-value="row.mode"
                    :options="forwardModes"
                    size="compact"
                    class="shrink-0"
                    :aria-label="t('sessions.editor.forwards.mode')"
                    @update:model-value="row.mode = $event as ForwardMode"
                  />
                  <input
                    v-model="row.bindHost"
                    type="text"
                    class="min-w-0 flex-1 rounded-md border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
                    :class="submitted && forwardProblem(row, 'bindHost') ? 'border-destructive' : 'border-border'"
                    :placeholder="t('sessions.editor.forwards.bindHostPlaceholder')"
                    :aria-label="t('sessions.editor.forwards.bindHost')"
                  />
                  <span class="text-muted-foreground">:</span>
                  <input
                    v-model="row.bindPort"
                    type="text"
                    inputmode="numeric"
                    class="w-14 rounded-md border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
                    :class="submitted && forwardProblem(row, 'bindPort') ? 'border-destructive' : 'border-border'"
                    :aria-label="t('sessions.editor.forwards.bindPort')"
                  />
                  <ArrowRight v-if="row.mode === 'local'" :size="12" class="shrink-0 text-muted-foreground" />
                  <span v-else class="shrink-0 font-mono text-[10px] text-muted-foreground">SOCKS5</span>
                  <input
                    v-if="row.mode === 'local'"
                    v-model="row.targetHost"
                    type="text"
                    class="min-w-0 flex-1 rounded-md border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
                    :class="submitted && forwardProblem(row, 'targetHost') ? 'border-destructive' : 'border-border'"
                    :placeholder="t('sessions.editor.forwards.targetHostPlaceholder')"
                    :aria-label="t('sessions.editor.forwards.targetHost')"
                  />
                  <span v-if="row.mode === 'local'" class="text-muted-foreground">:</span>
                  <input
                    v-if="row.mode === 'local'"
                    v-model="row.targetPort"
                    type="text"
                    inputmode="numeric"
                    class="w-14 rounded-md border bg-background px-2 py-1.5 font-mono text-xs focus:outline-none"
                    :class="submitted && forwardProblem(row, 'targetPort') ? 'border-destructive' : 'border-border'"
                    :placeholder="t('sessions.editor.forwards.targetPortPlaceholder')"
                    :aria-label="t('sessions.editor.forwards.targetPort')"
                  />
                  <button
                    type="button"
                    class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive"
                    :title="t('sessions.editor.forwards.remove')"
                    :aria-label="t('sessions.editor.forwards.remove')"
                    @click="editor.removeForward(row.id)"
                  >
                    <Trash2 :size="12" />
                  </button>
                </div>
                <template v-if="submitted">
                  <p v-for="message in forwardMessages(row)" :key="message" class="mt-0.5 text-[11px] text-destructive">
                    {{ message }}
                  </p>
                </template>
              </li>
            </ul>
          </section>

          <p v-if="problemText" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
            <span>{{ problemText.message }}</span>
            <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problemText.code }}</span>
          </p>

          <div class="flex justify-end gap-2 border-t border-border pt-4">
            <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
              {{ t("sessions.editor.cancel") }}
            </DialogClose>
            <button
              type="submit"
              class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
              :disabled="busy"
            >
              {{ t("sessions.editor.save") }}
            </button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>

  <!--
    The question about the key that is attached. It is asked after the editor is already open, so it
    is drawn over it — and the key's material is the one thing in this dialog that cannot be typed
    back in from memory.
  -->
  <ConfirmDialog
    v-if="forgetting"
    :open="true"
    :title="t('sessions.editor.key.forget')"
    :message="t('sessions.editor.key.forgetMessage', { name: editor.attachedKey?.displayName ?? '' })"
    :confirm-label="t('sessions.editor.key.forget')"
    danger
    @confirm="forgetKey()"
    @update:open="forgetting = false"
  />
</template>
