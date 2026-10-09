<script setup lang="ts">
/**
 * The keys this device holds: making one, importing one, and the way to stop holding one.
 *
 * Only the facts that identify a key are here — its name, its algorithm, its fingerprint, whether a
 * passphrase is held — because that is all the vault hands out about one. The list exists for the
 * same reason the trusted keys do: a key made or imported once should be reviewable later, and a key
 * that is no longer wanted should be removable without hunting for the session that uses it.
 *
 * **A key can be made here, and only its public half comes back.** That is what makes the page
 * useful on its own: the private half stays in the vault and authenticates from inside it, while the
 * line shown under a row is what somebody pastes into `authorized_keys` on a server. The line is
 * fetched when it is asked for rather than carried around, so a page nobody looks at moves no key
 * material anywhere.
 *
 * Forgetting says what it costs: the sessions using that key cannot connect until they are given
 * another one. The count comes from the sessions themselves rather than from a guess.
 */
import { Check, Copy, KeyRound, Plus, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import Select from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { useKeysStore } from "@/stores/keys";
import { useSessionsStore } from "@/stores/sessions";
import { KEY_ALGORITHMS, type KeyAlgorithm, type KeyInfo } from "@/types/session";

const { t } = useI18n();
const keys = useKeysStore();
const library = useSessionsStore();

const removing = ref<KeyInfo | null>(null);
const passphrase = ref("");
const comment = ref("");
/** The kind the dropdown is showing, which is the one a key would be made of. */
const algorithm = ref<KeyAlgorithm>(KEY_ALGORITHMS[0]);
/** Which key's public line is open, and the line itself once it has been fetched. */
const showing = ref<string | null>(null);
const publicLine = ref("");
const copied = ref(false);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = computed(() => (keys.failure ? describeShellError(translate, keys.failure) : null));

/** The three kinds as the dropdown draws them, in the order they are offered. */
const kinds = computed(() => KEY_ALGORITHMS.map((kind) => ({ value: kind, label: translate(`keys.kind.${kind}`) })));

/** How many saved sessions would lose their way in if this key went. */
function sessionsUsing(keyRef: string | undefined): number {
  if (!keyRef) {
    return 0;
  }
  return library.sessions.filter((session) => session.auth.key?.keyRef === keyRef).length;
}

/** Makes a key, and opens the line that has to go on a server — which is the point of making one. */
async function make(): Promise<void> {
  const created = await keys.generate(
    algorithm.value,
    comment.value,
    passphrase.value === "" ? null : passphrase.value,
  );
  if (created === null) {
    return;
  }

  // The line is already in hand, so it is shown rather than fetched again, and the row it belongs to
  // is the one the answer named. The comment is cleared with it: it travelled onto the key.
  showing.value = created.info.keyRef ?? null;
  publicLine.value = created.publicKey;
  comment.value = "";
}

/** Opens or closes the public line under one row, fetching it the first time. */
async function togglePublic(entry: KeyInfo): Promise<void> {
  if (showing.value === entry.keyRef) {
    showing.value = null;
    return;
  }

  publicLine.value = "";
  showing.value = entry.keyRef ?? null;
  if (!entry.keyRef) {
    return;
  }

  const line = await keys.publicKey(entry.keyRef);
  if (line !== null) {
    publicLine.value = line;
  }
}

/** Copying is a receipt: the button says it worked, and says nothing when the webview refuses. */
async function copyPublic(): Promise<void> {
  try {
    await navigator.clipboard.writeText(publicLine.value);
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, COPIED_MS);
  } catch {
    // A webview that refuses the clipboard is not worth an error banner over a copy button: the line
    // is on screen and selectable, which is why it is shown rather than only copied.
  }
}

/** How long the copy button says it copied something. */
const COPIED_MS = 1500;

onMounted(async () => {
  await keys.load();
  // The count in the question is about sessions, so they have to be here before it is asked.
  if (library.sessions.length === 0) {
    await library.load();
  }
});
</script>

<template>
  <!--
    Readings, inside a page that is otherwise chrome: an algorithm and a fingerprint are what a key
    is identified by, and both are copied out of here. The import field is text for the same reason
    every field is — it is taking typing — and the buttons stay buttons.
  -->
  <div class="space-y-2 select-text">
    <p v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <div class="flex items-center gap-2">
      <input
        id="key-passphrase"
        v-model="passphrase"
        type="password"
        autocomplete="off"
        class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs select-text focus:border-primary focus:outline-none"
        :placeholder="t('keys.passphrase')"
        :aria-label="t('keys.passphrase')"
      />
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent disabled:opacity-50"
        :disabled="keys.importing"
        @click="keys.importKey(t('keys.import'), passphrase === '' ? undefined : passphrase)"
      >
        <KeyRound :size="12" />
        {{ keys.importing ? t("keys.importing") : t("keys.import") }}
      </button>
    </div>

    <!--
      Making one, on the same row shape as importing one: a kind, a comment, and the button. The
      passphrase above is what protects either — it is the same question asked of two ways of getting
      a key into the vault.
    -->
    <div class="flex items-center gap-2">
      <Select
        class="shrink-0"
        :model-value="algorithm"
        :options="kinds"
        :aria-label="t('keys.makeKind')"
        :disabled="keys.making"
        trigger-class="w-40"
        @update:model-value="(value: string) => (algorithm = value as KeyAlgorithm)"
      />
      <input
        id="key-comment"
        v-model="comment"
        type="text"
        spellcheck="false"
        autocomplete="off"
        class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs select-text focus:border-primary focus:outline-none"
        :placeholder="t('keys.makeCommentPlaceholder')"
        :aria-label="t('keys.makeComment')"
        :disabled="keys.making"
      />
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent disabled:opacity-50"
        :disabled="keys.making"
        @click="make()"
      >
        <Plus :size="12" />
        {{ keys.making ? t("keys.making") : t("keys.make") }}
      </button>
    </div>
    <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("keys.makeHint") }}</p>

    <p v-if="keys.loaded && keys.entries.length === 0" class="text-xs text-muted-foreground">
      {{ t("keys.empty") }}
    </p>

    <div
      v-for="entry in keys.entries"
      :key="entry.keyRef ?? entry.fingerprint"
      class="rounded-md border border-border bg-muted/20 px-3 py-2.5"
    >
      <div class="flex items-start gap-3">
        <span class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-border bg-background">
          <KeyRound :size="12" class="text-muted-foreground" />
        </span>

        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-medium">{{ entry.displayName }}</p>
          <p class="mt-1 font-mono text-[11px] break-all text-muted-foreground">{{ entry.fingerprint }}</p>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{
              t("keys.algorithm", { algorithm: entry.algorithm, type: entry.hasPassphrase ? "passphrase" : "plain" })
            }}
            · {{ t("keys.usedBy", { count: sessionsUsing(entry.keyRef) }) }}
          </p>
        </div>

        <!-- The public half, which is the only thing about a key anybody needs to carry away. -->
        <button
          type="button"
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('keys.publicKey')"
          :aria-label="t('keys.publicKey')"
          :aria-expanded="showing !== null && showing === entry.keyRef"
          @click="togglePublic(entry)"
        >
          <Copy :size="12" />
        </button>

        <button
          type="button"
          class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
          :title="t('keys.forget')"
          :aria-label="t('keys.forget')"
          @click="removing = entry"
        >
          <Trash2 :size="12" />
        </button>
      </div>

      <div v-if="showing !== null && showing === entry.keyRef" class="mt-2 space-y-1 border-t border-border pt-2">
        <code class="block font-mono text-[11px] leading-relaxed break-all select-text">{{ publicLine }}</code>
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
            @click="copyPublic()"
          >
            <Check v-if="copied" :size="11" class="text-success" />
            <Copy v-else :size="11" />
            {{ copied ? t("keys.copiedPublic") : t("keys.copyPublic") }}
          </button>
          <span class="text-[11px] text-muted-foreground">{{ t("keys.publicHint") }}</span>
        </div>
      </div>
    </div>

    <ConfirmDialog
      v-if="removing"
      :open="true"
      :title="t('keys.forgetTitle')"
      :message="
        t('keys.forgetMessage', {
          name: removing.displayName,
          count: sessionsUsing(removing.keyRef),
        })
      "
      :confirm-label="t('keys.forgetConfirm')"
      danger
      @confirm="
        keys.forget(removing.keyRef ?? '');
        removing = null;
      "
      @update:open="removing = null"
    />
  </div>
</template>
