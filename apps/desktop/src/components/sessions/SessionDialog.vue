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
 */
import { KeyRound, LoaderCircle, Trash2, X } from "@lucide/vue";
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

import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useSessionsStore } from "@/stores/sessions";
import type { KeyInfo, Session, SecretInput } from "@/types/session";
import { AUTH_METHODS, PROTOCOLS, type AuthMethodId } from "@/types/generated/protocols";

const props = defineProps<{ open: boolean; session: Session | null }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const library = useSessionsStore();

/** Methods the transport refuses; the descriptors list them, this build does not implement them. */
const UNSUPPORTED_METHODS: readonly AuthMethodId[] = ["agent", "pageant"];

/** The protocol table holds one entry in this build; a new session starts on it. */
const DEFAULT_PROTOCOL = PROTOCOLS[0]?.id ?? "ssh";

/** What `TerminalSettings::default()` uses in Rust: the wire type has no "not set" for these. */
const DEFAULT_TERMINAL = { terminalType: "xterm-256color", keepaliveSeconds: 30 };

const name = ref("");
const host = ref("");
const port = ref("");
const user = ref("");
const groupId = ref("");
const method = ref<AuthMethodId>("password");
const secret = ref("");
const attachedKey = ref<KeyInfo | null>(null);
const keyHasPassphrase = ref(false);
const passphrase = ref("");
const submitted = ref(false);
const busy = ref(false);
const keyBusy = ref(false);
const problem = ref<ShellError | null>(null);

const title = computed(() => (props.session ? t("sessions.editor.titleEdit") : t("sessions.editor.titleNew")));
const storedSecret = computed(() => props.session?.auth.hasSecret ?? false);

const portNumber = computed(() => Number(port.value.trim()));

/** Every rule is reported next to the field it belongs to, which is why they are collected here. */
const errors = computed<Record<string, string>>(() => {
  const found: Record<string, string> = {};
  if (name.value.trim() === "") {
    found.name = t("sessions.editor.validation.name");
  }
  if (host.value.trim() === "") {
    found.host = t("sessions.editor.validation.host");
  }
  if (user.value.trim() === "") {
    found.user = t("sessions.editor.validation.user");
  }
  if (!Number.isInteger(portNumber.value) || portNumber.value < 1 || portNumber.value > 65535) {
    found.port = t("sessions.editor.validation.port");
  }
  if (method.value === "publicKey" && !attachedKey.value?.keyRef) {
    found.key = t("sessions.editor.validation.key");
  }
  return found;
});

const problemText = computed(() => (problem.value ? describeShellError(translate, problem.value) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

watch(
  () => [props.open, props.session] as const,
  ([open]) => {
    if (open) {
      fill();
    }
  },
  { immediate: true },
);

function fill(): void {
  const session = props.session;
  name.value = session?.name ?? "";
  host.value = session?.host ?? "";
  port.value = String(session?.port ?? PROTOCOLS[0]?.defaultPort ?? "");
  user.value = session?.user ?? "";
  groupId.value = session?.groupId ?? "";
  method.value = session?.auth.method ?? "password";
  attachedKey.value = session?.auth.key ?? null;
  secret.value = "";
  passphrase.value = "";
  keyHasPassphrase.value = false;
  submitted.value = false;
  busy.value = false;
  keyBusy.value = false;
  problem.value = null;
}

/**
 * A secret travels only when one was typed: an empty field means "keep the stored one", which is
 * not the same as blank, and there is deliberately no way to say "remove it".
 */
function secretInput(): SecretInput | undefined {
  return secret.value === "" ? undefined : { value: secret.value };
}

function draft(): Session {
  const existing = props.session;
  return {
    id: existing?.id ?? crypto.randomUUID(),
    name: name.value.trim(),
    groupId: groupId.value === "" ? undefined : groupId.value,
    protocol: existing?.protocol ?? DEFAULT_PROTOCOL,
    host: host.value.trim(),
    port: portNumber.value,
    user: user.value.trim(),
    auth: {
      method: method.value,
      key: attachedKey.value ?? undefined,
      hasSecret: existing?.auth.hasSecret ?? false,
    },
    terminal: existing?.terminal ?? DEFAULT_TERMINAL,
    updatedAt: Math.floor(Date.now() / 1000),
  };
}

async function save(): Promise<void> {
  submitted.value = true;
  problem.value = null;
  if (Object.keys(errors.value).length > 0) {
    return;
  }

  busy.value = true;
  try {
    await library.save(draft(), secretInput());
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
      passphrase.value === "" ? undefined : passphrase.value,
    );
    if (info) {
      attachedKey.value = info;
    }
    passphrase.value = "";
  } catch (error) {
    problem.value = asShellError(error, "key_import");
  } finally {
    keyBusy.value = false;
  }
}

async function forgetKey(): Promise<void> {
  const info = attachedKey.value;
  problem.value = null;
  if (!info?.keyRef) {
    attachedKey.value = null;
    return;
  }
  try {
    await backend.keys.forget(info.keyRef);
    attachedKey.value = null;
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
                v-model="name"
                type="text"
                class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
                :class="submitted && errors.name ? 'border-destructive' : 'border-border'"
                :placeholder="t('sessions.editor.namePlaceholder')"
              />
              <p v-if="submitted && errors.name" class="mt-1 text-[11px] text-destructive">{{ errors.name }}</p>
            </div>

            <div>
              <label class="block text-xs font-medium" for="session-group">{{ t("sessions.editor.group") }}</label>
              <select
                id="session-group"
                v-model="groupId"
                class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
              >
                <option value="">{{ t("sessions.editor.groupNone") }}</option>
                <option v-for="group in library.groups" :key="group.id" :value="group.id">{{ group.name }}</option>
              </select>
            </div>

            <div class="sm:col-span-2">
              <label class="block text-xs font-medium" for="session-host">{{ t("sessions.editor.host") }}</label>
              <input
                id="session-host"
                v-model="host"
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
                v-model="port"
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
                v-model="user"
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
            <select
              id="session-method"
              v-model="method"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
            >
              <option
                v-for="option in AUTH_METHODS"
                :key="option.id"
                :value="option.id"
                :disabled="UNSUPPORTED_METHODS.includes(option.id)"
              >
                {{ t(option.labelKey) }}
              </option>
            </select>
            <p class="mt-1 text-[11px] text-muted-foreground">{{ t("sessions.editor.methodUnsupported") }}</p>

            <div v-if="method === 'password' || method === 'keyboardInteractive'" class="mt-3">
              <label class="block text-xs font-medium" for="session-secret">{{
                t("sessions.editor.secret.label")
              }}</label>
              <input
                id="session-secret"
                v-model="secret"
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

            <div v-else-if="method === 'publicKey'" class="mt-3 space-y-3">
              <div v-if="attachedKey" class="rounded-md border border-border bg-background px-3 py-3">
                <p class="flex items-center gap-2 text-xs font-medium">
                  <KeyRound :size="13" class="text-muted-foreground" />
                  {{ t("sessions.editor.key.attached") }}
                </p>
                <dl class="mt-2 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-[11px]">
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.name") }}</dt>
                  <dd class="font-mono break-all">{{ attachedKey.displayName }}</dd>
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.algorithm") }}</dt>
                  <dd class="font-mono">{{ attachedKey.algorithm }}</dd>
                  <dt class="text-muted-foreground">{{ t("sessions.editor.key.fingerprint") }}</dt>
                  <dd class="font-mono break-all">{{ attachedKey.fingerprint }}</dd>
                </dl>
                <p class="mt-2 text-[11px] text-muted-foreground">
                  {{
                    attachedKey.hasPassphrase
                      ? t("sessions.editor.key.passphraseStored")
                      : t("sessions.editor.key.noPassphraseStored")
                  }}
                </p>
                <button
                  type="button"
                  class="mt-2 flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
                  @click="forgetKey()"
                >
                  <Trash2 :size="12" />
                  {{ t("sessions.editor.key.forget") }}
                </button>
              </div>
              <p v-else class="text-[11px] text-muted-foreground">{{ t("sessions.editor.key.none") }}</p>

              <label class="flex items-center gap-2 text-xs">
                <input v-model="keyHasPassphrase" type="checkbox" class="size-3.5 rounded-sm border-border" />
                {{ t("sessions.editor.key.hasPassphrase") }}
              </label>

              <div v-if="keyHasPassphrase">
                <input
                  v-model="passphrase"
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
</template>
