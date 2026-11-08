<script setup lang="ts">
/**
 * One remote entry's facts, and the one fact that can be changed: its permission bits.
 *
 * The list already shows these columns; this is where they can be read without the column widths
 * deciding what fits, and where `chmod` has the same nine boxes the focused permissions dialog has.
 */
import { X } from "@lucide/vue";
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

import PermissionMatrix from "@/components/files/PermissionMatrix.vue";
import { describeShellError } from "@/i18n/errors";
import { formatModified, formatSize } from "@/lib/files/format";
import { octalMode, parseOctalMode } from "@/lib/files/mode";
import { useFilesStore } from "@/stores/files";
import type { SftpEntry } from "@/types/session";

const props = defineProps<{ open: boolean; entry: SftpEntry | null; handle: string }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const files = useFilesStore();

const modeText = ref("");
const appliedMode = ref<number | undefined>(undefined);
const busy = ref(false);
const saved = ref(false);
const attempted = ref(false);
const modeField = ref<HTMLInputElement | null>(null);

watch(
  () => [props.open, props.entry?.path] as const,
  () => {
    if (!props.open || !props.entry) {
      return;
    }
    modeText.value = props.entry.mode === undefined ? "" : octalMode(props.entry.mode);
    appliedMode.value = props.entry.mode;
    saved.value = false;
    attempted.value = false;
  },
  { immediate: true },
);

const parsedMode = computed(() => parseOctalMode(modeText.value));
const shownMode = computed(() => {
  const mode = appliedMode.value;
  return mode === undefined ? "—" : octalMode(mode);
});
/** `owner:group (uid:gid)`, with the numbers left off when the server did not name either side. */
const ownerDisplay = computed(() => {
  const entry = props.entry;
  if (!entry) {
    return "—";
  }
  const owner = entry.owner ?? "—";
  const group = entry.group ?? "—";
  const label = `${owner}:${group}`;
  if (entry.uid === undefined || entry.gid === undefined) {
    return label;
  }
  const ownerIsUid = owner === String(entry.uid);
  const groupIsGid = group === String(entry.gid);
  return ownerIsUid && groupIsGid ? label : `${label} (${entry.uid}:${entry.gid})`;
});
const failure = computed(() => {
  if (!attempted.value) {
    return null;
  }
  const browser = files.browser(props.handle);
  return browser?.failure ? describeShellError((key, params) => t(key, params ?? {}), browser.failure) : null;
});

function focusMode(event: Event): void {
  event.preventDefault();
  modeField.value?.focus();
  modeField.value?.select();
}

async function apply(): Promise<void> {
  const entry = props.entry;
  const mode = parsedMode.value;
  if (!entry || mode === null || busy.value) {
    return;
  }
  busy.value = true;
  saved.value = false;
  const worked = await files.setPermissions(props.handle, entry.path, mode);
  busy.value = false;
  attempted.value = true;
  if (worked) {
    appliedMode.value = mode;
    saved.value = true;
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(32rem,calc(100vw-2rem))] -translate-y-1/2 -translate-x-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @open-auto-focus="focusMode"
        @escape-key-down="busy && $event.preventDefault()"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">
            {{ t("files.properties.title") }}<template v-if="props.entry"> · {{ props.entry.name }}</template>
          </DialogTitle>
          <DialogDescription class="sr-only">{{ props.entry?.path ?? "" }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('files.properties.close')"
            :disabled="busy"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <div v-if="props.entry" class="space-y-4 px-4 py-4 text-xs">
          <dl class="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1.5">
            <dt class="text-muted-foreground">{{ t("files.properties.path") }}</dt>
            <dd class="truncate font-mono text-[11px] select-text" :title="props.entry.path">{{ props.entry.path }}</dd>

            <dt class="text-muted-foreground">{{ t("files.properties.kind") }}</dt>
            <dd>{{ t(`files.kind.${props.entry.kind}`) }}</dd>

            <dt class="text-muted-foreground">{{ t("files.properties.size") }}</dt>
            <dd class="font-mono text-[11px]">
              {{ props.entry.kind === "directory" ? "–" : formatSize(props.entry.size) }}
            </dd>

            <dt class="text-muted-foreground">{{ t("files.properties.permissions") }}</dt>
            <dd class="font-mono text-[11px]">{{ shownMode }}</dd>

            <dt class="text-muted-foreground">{{ t("files.properties.owner") }}</dt>
            <dd class="font-mono text-[11px]">{{ ownerDisplay }}</dd>

            <dt class="text-muted-foreground">{{ t("files.properties.modified") }}</dt>
            <dd>{{ props.entry.modifiedAt === undefined ? "–" : formatModified(props.entry.modifiedAt) }}</dd>

            <template v-if="props.entry.kind === 'symlink' && props.entry.linkTarget">
              <dt class="text-muted-foreground">{{ t("files.properties.linkTarget") }}</dt>
              <dd class="truncate font-mono text-[11px] select-text" :title="props.entry.linkTarget">
                {{ props.entry.linkTarget }}
              </dd>
            </template>
          </dl>

          <PermissionMatrix
            :model-value="modeText"
            :label="t('files.properties.permissions')"
            @update:model-value="modeText = $event"
          />

          <div class="space-y-1.5">
            <label class="block text-xs font-medium" for="file-properties-mode">
              {{ t("files.properties.permissions") }}
            </label>
            <input
              id="file-properties-mode"
              ref="modeField"
              v-model="modeText"
              type="text"
              spellcheck="false"
              inputmode="numeric"
              class="w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm select-text focus:border-primary focus:outline-none"
              :class="parsedMode === null ? 'border-destructive' : ''"
              :aria-label="t('files.properties.permissions')"
              :aria-invalid="parsedMode === null"
            />
            <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("files.chmod.hint") }}</p>
            <p v-if="parsedMode === null" class="text-[11px] text-destructive">
              {{ t("files.properties.invalid") }}
            </p>
            <p v-if="saved" class="text-[11px] text-muted-foreground">{{ t("files.properties.saved") }}</p>
            <p v-if="failure" class="text-[11px] text-destructive">{{ failure.message }}</p>
          </div>
        </div>

        <footer
          class="flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="busy"
            @click="emit('update:open', false)"
          >
            {{ t("files.properties.close") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="busy || parsedMode === null || !props.entry"
            @click="apply()"
          >
            {{ t("files.properties.apply") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
