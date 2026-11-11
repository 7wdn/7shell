<script setup lang="ts">
/**
 * The assistant's settings: where its questions go, and how much terminal travels with one.
 *
 * Three rules, each of them about something this page could get wrong:
 *
 * 1. **A key is written and never read back.** The field is a password box that is empty every time
 *    the form opens, `hasKey` is the only statement the page makes about the vault, and a locked
 *    vault says "cannot tell" rather than "none" — that is the whole difference between `null` and
 *    `false`, and it is drawn here.
 * 2. **The list is the settings file; the key state is the host.** A provider's name, address and
 *    model are edited here and written with the page's Apply; whether a key is stored comes from
 *    `ai_providers`, which is asked again after every save and every key change.
 * 3. **Testing is separate from saving.** A connection test happens against what the host already
 *    holds, so the button lives on the saved row rather than in the form: a test against a half-typed
 *    address would answer about something nobody stored.
 */
import { Check, CircleAlert, LoaderCircle, Pencil, Plus, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import { describeShellError } from "@/i18n/errors";
import {
  PROVIDER_DEFAULT_BASE_URL,
  PROVIDER_KEY_STATE_KEYS,
  PROVIDER_KIND_HINT_KEYS,
  PROVIDER_KIND_LABEL_KEYS,
} from "@/lib/assistant/labels";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useAssistantStore } from "@/stores/assistant";
import { usePreferencesStore } from "@/stores/preferences";
import {
  AI_CONTEXT_LINES_MAX,
  AI_CONTEXT_LINES_MIN,
  AI_MAX_OUTPUT_TOKENS_DEFAULT,
  AI_SESSION_RETENTION_DAYS_DEFAULT,
  AI_SESSION_RETENTION_DAYS_MAX,
  AI_SESSION_RETENTION_DAYS_MIN,
  type AiProviderConfig,
  type AiProviderKind,
} from "@/types/app";
import { AI_MAX_OUTPUT_TOKENS_MAX, AI_MAX_OUTPUT_TOKENS_MIN, type AiProviderView } from "@/types/ai";

const { t } = useI18n();
const preferences = usePreferencesStore();
const assistant = useAssistantStore();

/**
 * The protocols the form offers, in the order the dropdown lists them.
 *
 * Their labels, hints and default addresses are the shared tables in `lib/assistant/labels.ts`, so a
 * protocol is described once and every surface draws the same words for it.
 *
 * The Responses API is offered beside Chat Completions: it is OpenAI's other endpoint, not a second
 * spelling of the same one, so it is a choice of its own. A gateway that does not serve
 * `/v1/responses` answers with the provider's refusal, which is what the test button reports.
 */
const KINDS: readonly AiProviderKind[] = ["anthropic", "openai", "openaiResponses", "local"];

/** What the form is editing, including the key — which is the one field that is never read back. */
interface Draft {
  id: string;
  isNew: boolean;
  name: string;
  kind: AiProviderKind;
  baseUrl: string;
  model: string;
  maxOutputTokens: number;
  key: string;
}

const draft = ref<Draft | null>(null);
const saving = ref(false);
const notice = ref<"" | "saved" | "keySaved" | "keyForgotten">("");
const failure = ref<ShellError | null>(null);
/** The row a connection test is running for, and what the last one answered. */
const testing = ref<string | null>(null);
const testResult = ref<{ id: string; ok: boolean; text: string } | null>(null);
/** The provider whose removal is being asked about. */
const removing = ref<AiProviderView | null>(null);

const problem = computed(() => (failure.value ? describeShellError(translate, failure.value) : null));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** The views the host answers with, by id: the only place a key state comes from. */
const viewsById = computed(() => new Map(assistant.providers.map((view) => [view.id, view])));

function keyState(provider: AiProviderConfig): string {
  const hasKey = viewsById.value.get(provider.id)?.hasKey;
  if (hasKey === true) {
    return t(PROVIDER_KEY_STATE_KEYS.saved);
  }
  return t(hasKey === false ? PROVIDER_KEY_STATE_KEYS.missing : PROVIDER_KEY_STATE_KEYS.locked);
}

function keyIsMissing(provider: AiProviderConfig): boolean {
  return viewsById.value.get(provider.id)?.hasKey === false;
}

const kindOptions = computed(() => KINDS.map((kind) => ({ value: kind, label: t(PROVIDER_KIND_LABEL_KEYS[kind]) })));

const providerOptions = computed(() =>
  preferences.aiProviders.map((provider) => ({
    value: provider.id,
    label: provider.name,
    detail: t(PROVIDER_KIND_LABEL_KEYS[provider.kind]),
  })),
);

const chosenProvider = computed(() => preferences.aiDefaultProviderId ?? "");

const kindHint = computed(() => (draft.value ? t(PROVIDER_KIND_HINT_KEYS[draft.value.kind]) : ""));

const canSaveDraft = computed(() => {
  const current = draft.value;
  if (!current) {
    return false;
  }
  return (
    current.name.trim() !== "" &&
    current.model.trim() !== "" &&
    current.baseUrl.trim() !== "" &&
    current.maxOutputTokens >= AI_MAX_OUTPUT_TOKENS_MIN
  );
});

/** A new identifier: opaque, stable, and what the vault files this provider's key under. */
function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `provider-${Date.now()}-${Math.round(Math.random() * 1_000_000)}`;
}

function startNew(): void {
  notice.value = "";
  failure.value = null;
  draft.value = {
    id: newId(),
    isNew: true,
    name: "",
    kind: "anthropic",
    baseUrl: PROVIDER_DEFAULT_BASE_URL.anthropic,
    model: "",
    maxOutputTokens: AI_MAX_OUTPUT_TOKENS_DEFAULT,
    key: "",
  };
}

function startEdit(provider: AiProviderConfig): void {
  notice.value = "";
  failure.value = null;
  draft.value = {
    id: provider.id,
    isNew: false,
    name: provider.name,
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    model: provider.model,
    maxOutputTokens: provider.maxOutputTokens,
    // Empty on purpose: the stored key is never read back, and an empty box keeps it.
    key: "",
  };
}

function cancelDraft(): void {
  draft.value = null;
}

/** A protocol change rewrites the address only while it is still the previous protocol's default. */
function chooseKind(kind: AiProviderKind): void {
  const current = draft.value;
  if (!current) {
    return;
  }
  const previousDefault = PROVIDER_DEFAULT_BASE_URL[current.kind];
  current.kind = kind;
  if (current.baseUrl.trim() === "" || current.baseUrl.trim() === previousDefault) {
    current.baseUrl = PROVIDER_DEFAULT_BASE_URL[kind];
  }
}

function typedNumber(event: Event, fallback: number): number {
  const value = Number((event.target as HTMLInputElement).value || String(fallback));
  return Number.isFinite(value) ? Math.round(value) : fallback;
}

/**
 * Writes the draft.
 *
 * The key goes in its own call, after the configuration: the host files it against the id, and an id
 * that only exists in the settings file once this has been written would be a key stored for nobody.
 * A failure on either half leaves the form up with the reason, because the two halves are one thing
 * in the person's head.
 */
async function saveDraft(): Promise<void> {
  const current = draft.value;
  if (!current || saving.value || !canSaveDraft.value) {
    return;
  }
  saving.value = true;
  failure.value = null;
  notice.value = "";

  const config: AiProviderConfig = {
    id: current.id,
    name: current.name.trim(),
    kind: current.kind,
    baseUrl: current.baseUrl.trim(),
    model: current.model.trim(),
    maxOutputTokens: Math.min(Math.max(current.maxOutputTokens, AI_MAX_OUTPUT_TOKENS_MIN), AI_MAX_OUTPUT_TOKENS_MAX),
  };

  const exists = preferences.aiProviders.some((entry) => entry.id === config.id);
  preferences.setAiProviders(
    exists
      ? preferences.aiProviders.map((entry) => (entry.id === config.id ? config : entry))
      : [...preferences.aiProviders, config],
  );

  try {
    // The key command resolves the provider from the settings file, so the configuration has to be
    // on disk before the key is filed against it. Committing first also means a key is never stored
    // for a provider that was not saved.
    if (!(await preferences.commit())) {
      return;
    }
    if (current.key.trim() !== "") {
      await backend.ai.setKey(config.id, current.key);
    }
    draft.value = null;
    notice.value = current.key.trim() === "" ? "saved" : "keySaved";
  } catch (error) {
    failure.value = asShellError(error, "ai_provider_key_set");
  } finally {
    // The list is re-read either way: it is the host's answer about the keys, not this page's.
    await assistant.refreshProviders();
    saving.value = false;
  }
}

/** Forgets one key without touching the configuration it belongs to. */
async function forgetKey(provider: AiProviderConfig): Promise<void> {
  failure.value = null;
  notice.value = "";
  try {
    await backend.ai.setKey(provider.id, null);
    notice.value = "keyForgotten";
  } catch (error) {
    failure.value = asShellError(error, "ai_provider_key_set");
  } finally {
    await assistant.refreshProviders();
  }
}

/** Asks the provider whether it answers, with what the host already holds. */
async function test(provider: AiProviderConfig): Promise<void> {
  if (testing.value !== null) {
    return;
  }
  testing.value = provider.id;
  testResult.value = null;
  failure.value = null;
  try {
    const result = await backend.ai.test(provider.id);
    const text = result.ok
      ? t("ai.settings.providers.testOk")
      : result.messageKey
        ? translate(result.messageKey, { ...result.messageParams })
        : t("ai.settings.providers.testFailed");
    testResult.value = { id: provider.id, ok: result.ok, text };
  } catch (error) {
    failure.value = asShellError(error, "ai_provider_test");
  } finally {
    testing.value = null;
  }
}

/**
 * Removes a provider and the key kept for it.
 *
 * The host goes first: a row that vanished from the list while the vault still held its key would
 * leave a secret nobody can see and nobody can delete.
 */
async function confirmRemove(): Promise<void> {
  const provider = removing.value;
  removing.value = null;
  if (!provider) {
    return;
  }
  failure.value = null;
  try {
    await backend.ai.remove(provider.id);
  } catch (error) {
    failure.value = asShellError(error, "ai_provider_remove");
    return;
  }
  preferences.setAiProviders(preferences.aiProviders.filter((entry) => entry.id !== provider.id));
  await preferences.commit();
  await assistant.refreshProviders();
}

function chooseProvider(id: string): void {
  preferences.setAiDefaultProviderId(id === "" ? null : id);
}

/** Writes what the page has chosen, and re-reads the host's answer about the keys. */
async function savePage(): Promise<void> {
  notice.value = "";
  failure.value = null;
  if (await preferences.commit()) {
    notice.value = "saved";
    await assistant.refreshProviders();
  }
}

onMounted(() => {
  void assistant.refreshProviders();
});
</script>

<template>
  <section class="space-y-3">
    <p
      v-if="problem"
      data-ai-settings-problem
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
    >
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <p class="text-xs leading-relaxed text-muted-foreground">{{ t("ai.settings.description") }}</p>

    <!-- The providers: one card each, and the form that edits one of them. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h4 class="text-sm font-medium">{{ t("ai.settings.providers.title") }}</h4>
          <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">
            {{ t("ai.settings.providers.description") }}
          </p>
        </div>
        <button
          type="button"
          data-ai-provider-add
          class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="draft !== null"
          @click="startNew()"
        >
          <Plus :size="12" />
          {{ t("ai.settings.providers.add") }}
        </button>
      </div>

      <p v-if="preferences.aiProviders.length === 0 && draft === null" class="text-xs text-muted-foreground">
        {{ t("ai.settings.providers.empty") }}
      </p>

      <ul class="space-y-1.5">
        <li
          v-for="provider in preferences.aiProviders"
          :key="provider.id"
          class="rounded-md border border-border bg-muted/20 px-3 py-2"
        >
          <div class="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span class="text-xs font-medium">{{ provider.name }}</span>
            <span class="text-[11px] text-muted-foreground">{{ t(PROVIDER_KIND_LABEL_KEYS[provider.kind]) }}</span>
            <span class="font-mono text-[11px] text-muted-foreground">{{ provider.model }}</span>
            <span class="text-[11px]" :class="keyIsMissing(provider) ? 'text-warning' : 'text-muted-foreground'">
              {{ keyState(provider) }}
            </span>
          </div>
          <p class="mt-0.5 font-mono text-[11px] break-all text-muted-foreground">{{ provider.baseUrl }}</p>

          <div class="mt-1 flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="testing !== null || draft !== null"
              @click="test(provider)"
            >
              <LoaderCircle v-if="testing === provider.id" :size="10" class="animate-spin" />
              {{ testing === provider.id ? t("ai.settings.providers.testing") : t("ai.settings.providers.test") }}
            </button>
            <button
              type="button"
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="draft !== null"
              @click="startEdit(provider)"
            >
              <Pencil :size="10" />
              {{ t("ai.settings.providers.edit") }}
            </button>
            <button
              type="button"
              class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
              :disabled="viewsById.get(provider.id)?.hasKey !== true"
              @click="forgetKey(provider)"
            >
              <Trash2 :size="10" />
              {{ t("ai.settings.providers.keyClear") }}
            </button>
            <button
              type="button"
              class="ml-auto flex items-center gap-1 rounded-md border border-destructive/40 px-2 py-0.5 text-[11px] text-destructive transition-colors hover:bg-destructive/10 disabled:pointer-events-none disabled:opacity-50"
              :disabled="draft !== null"
              @click="removing = viewsById.get(provider.id) ?? { ...provider, hasKey: null }"
            >
              <Trash2 :size="10" />
              {{ t("ai.settings.providers.remove") }}
            </button>
          </div>

          <p
            v-if="testResult && testResult.id === provider.id"
            class="mt-1 flex items-start gap-1 text-[11px]"
            :class="testResult.ok ? 'text-success' : 'text-destructive'"
          >
            <Check v-if="testResult.ok" :size="11" class="mt-0.5 shrink-0" />
            <CircleAlert v-else :size="11" class="mt-0.5 shrink-0" />
            <span>{{ testResult.text }}</span>
          </p>
        </li>
      </ul>

      <!-- The form: one provider at a time, with the key box always empty. -->
      <div v-if="draft" data-ai-provider-form class="space-y-2 rounded-md border border-primary/40 px-3 py-2.5">
        <h5 class="text-xs font-medium">
          {{ t(draft.isNew ? "ai.settings.new.title" : "ai.settings.editTitle") }}
        </h5>

        <div class="grid gap-2 sm:grid-cols-2">
          <div>
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-name">
              {{ t("ai.settings.providers.name") }}
            </label>
            <input
              id="ai-provider-name"
              v-model="draft.name"
              type="text"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
              :placeholder="t('ai.settings.providers.name')"
            />
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-kind">
              {{ t("ai.settings.providers.kind") }}
            </label>
            <div class="mt-0.5">
              <Select
                id="ai-provider-kind"
                :model-value="draft.kind"
                :options="kindOptions"
                :aria-label="t('ai.settings.providers.kind')"
                @update:model-value="(value) => chooseKind(value as AiProviderKind)"
              />
            </div>
          </div>
          <div class="sm:col-span-2">
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-base-url">
              {{ t("ai.settings.providers.baseUrl") }}
            </label>
            <input
              id="ai-provider-base-url"
              v-model="draft.baseUrl"
              type="url"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="PROVIDER_DEFAULT_BASE_URL[draft.kind]"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
              {{ t("ai.settings.providers.baseUrlHint") }}
            </p>
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-model">
              {{ t("ai.settings.providers.model") }}
            </label>
            <input
              id="ai-provider-model"
              v-model="draft.model"
              type="text"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="t('ai.settings.providers.model')"
            />
          </div>
          <div>
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-max-output">
              {{ t("ai.settings.providers.maxOutput") }}
            </label>
            <input
              id="ai-provider-max-output"
              type="number"
              :min="AI_MAX_OUTPUT_TOKENS_MIN"
              :max="AI_MAX_OUTPUT_TOKENS_MAX"
              step="1"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
              :value="draft.maxOutputTokens"
              @input="draft.maxOutputTokens = typedNumber($event, AI_MAX_OUTPUT_TOKENS_DEFAULT)"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
              {{ t("ai.settings.providers.maxOutputHint") }}
            </p>
          </div>
          <div class="sm:col-span-2">
            <label class="block text-[11px] text-muted-foreground" for="ai-provider-key">
              {{ t("ai.settings.providers.key") }}
            </label>
            <input
              id="ai-provider-key"
              v-model="draft.key"
              type="password"
              autocomplete="off"
              spellcheck="false"
              class="mt-0.5 w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none select-text"
              :placeholder="t('ai.settings.providers.key')"
              @keydown.enter.prevent="saveDraft()"
            />
            <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
              {{ t("ai.settings.providers.keyHint") }}
            </p>
          </div>
        </div>

        <p class="text-[11px] leading-relaxed text-muted-foreground">{{ kindHint }}</p>

        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
            :disabled="!canSaveDraft || saving"
            @click="saveDraft()"
          >
            {{ t("ai.settings.providers.save") }}
          </button>
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1 text-xs transition-colors hover:bg-accent"
            :disabled="saving"
            @click="cancelDraft()"
          >
            {{ t("ai.settings.providers.cancel") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Which provider a new question goes to. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <label class="block text-xs font-medium" for="ai-active-provider">
        {{ t("ai.settings.providers.active") }}
      </label>
      <Select
        id="ai-active-provider"
        :model-value="chosenProvider"
        :options="providerOptions"
        :placeholder="t('ai.settings.providers.activeAutomatic')"
        :aria-label="t('ai.settings.providers.active')"
        @update:model-value="chooseProvider"
      />
      <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("ai.settings.providers.activeHint") }}</p>
    </div>

    <!-- How much terminal travels, which is the same two fields the panel shows under the chat. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <h4 class="text-sm font-medium">{{ t("ai.settings.context.title") }}</h4>
      <div class="flex items-center justify-between gap-3">
        <span class="text-xs">{{ t("ai.settings.context.enabled") }}</span>
        <Switch
          :model-value="preferences.aiContextEnabled"
          :aria-label="t('ai.settings.context.enabled')"
          @update:model-value="(value) => preferences.setAiContextEnabled(value)"
        />
      </div>
      <div class="flex items-center gap-2">
        <label class="text-xs" for="ai-context-lines">{{ t("ai.settings.context.lines") }}</label>
        <input
          id="ai-context-lines"
          type="number"
          :min="AI_CONTEXT_LINES_MIN"
          :max="AI_CONTEXT_LINES_MAX"
          step="1"
          class="w-20 rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
          :value="preferences.aiContextLines"
          @input="preferences.setAiContextLines(typedNumber($event, 0))"
        />
        <span class="min-w-0 flex-1 text-[11px] leading-relaxed text-muted-foreground">
          {{ t("ai.context.linesHint") }}
        </span>
      </div>
    </div>

    <!-- How long a stored conversation is kept. A setting, not a cleanup button: the host prunes. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <h4 class="text-sm font-medium">{{ t("ai.settings.sessions.title") }}</h4>
      <p class="text-xs leading-relaxed text-muted-foreground">{{ t("ai.settings.sessions.description") }}</p>
      <div class="flex items-center gap-2">
        <label class="text-xs" for="ai-retention-days">{{ t("ai.settings.sessions.retentionDays") }}</label>
        <input
          id="ai-retention-days"
          data-ai-retention-days
          type="number"
          :min="AI_SESSION_RETENTION_DAYS_MIN"
          :max="AI_SESSION_RETENTION_DAYS_MAX"
          step="1"
          class="w-20 rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none select-text"
          :value="preferences.aiSessionRetentionDays"
          @input="preferences.setAiSessionRetentionDays(typedNumber($event, AI_SESSION_RETENTION_DAYS_DEFAULT))"
        />
        <span class="text-[11px] text-muted-foreground">{{ t("ai.settings.sessions.unitDays") }}</span>
      </div>
      <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("ai.settings.sessions.retentionDaysHint") }}</p>
    </div>

    <!-- What leaves the machine, said once where the configuration is. -->
    <div class="space-y-1 rounded-md border border-border px-3 py-3">
      <h4 class="text-sm font-medium">{{ t("ai.settings.privacy.title") }}</h4>
      <p class="text-xs leading-relaxed text-muted-foreground">{{ t("ai.settings.privacy.body") }}</p>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
        @click="savePage()"
      >
        {{ t("ai.settings.context.save") }}
      </button>
      <span v-if="notice === 'saved'" class="text-xs text-muted-foreground">{{ t("ai.settings.context.saved") }}</span>
      <span v-else-if="notice === 'keySaved'" class="text-xs text-success">{{
        t("ai.settings.providers.keySaved")
      }}</span>
      <span v-else-if="notice === 'keyForgotten'" class="text-xs text-muted-foreground">
        {{ t("ai.settings.providers.keyMissing") }}
      </span>
    </div>

    <ConfirmDialog
      v-if="removing"
      :open="true"
      :title="t('ai.settings.providers.removeTitle')"
      :message="t('ai.settings.providers.removeMessage')"
      :details="[removing.name]"
      :confirm-label="t('ai.settings.providers.remove')"
      danger
      @confirm="confirmRemove()"
      @update:open="removing = null"
    />
  </section>
</template>
