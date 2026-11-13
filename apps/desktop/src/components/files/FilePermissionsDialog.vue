<script setup lang="ts">
/**
 * `chmod` for one entry: the nine boxes and the octal draft, with nothing else on screen.
 *
 * The properties dialog shows the same editor beside the readings; this is the short way in when the
 * permissions are the only thing being changed.
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
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { octalMode, parseOctalMode } from "@/lib/files/mode";
import { useFilesStore } from "@/stores/files";
import type { SftpEntry } from "@/types/session";

const props = defineProps<{ open: boolean; entry: SftpEntry | null; handle: string }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const files = useFilesStore();
const { toast } = useToast();

const modeText = ref("");
const busy = ref(false);
const attempted = ref(false);
const modeField = ref<HTMLInputElement | null>(null);

watch(
  () => [props.open, props.entry?.path] as const,
  () => {
    if (!props.open || !props.entry) {
      return;
    }
    modeText.value = props.entry.mode === undefined ? "" : octalMode(props.entry.mode);
    attempted.value = false;
  },
  { immediate: true },
);

const parsedMode = computed(() => parseOctalMode(modeText.value));
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

async function confirm(): Promise<void> {
  const entry = props.entry;
  const mode = parsedMode.value;
  if (!entry || mode === null || busy.value) {
    return;
  }
  busy.value = true;
  const worked = await files.setPermissions(props.handle, entry.path, mode);
  busy.value = false;
  attempted.value = true;
  if (!worked) {
    return;
  }
  toast(t("files.chmod.updated"), "receipt");
  emit("update:open", false);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @open-auto-focus="focusMode"
        @escape-key-down="busy && $event.preventDefault()"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">
            {{ t("files.chmod.title") }}<template v-if="props.entry"> · {{ props.entry.name }}</template>
          </DialogTitle>
          <DialogDescription class="sr-only">{{ props.entry?.path ?? "" }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('files.chmod.cancel')"
            :disabled="busy"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <div class="space-y-3 px-4 py-4">
          <PermissionMatrix
            :model-value="modeText"
            :label="t('files.chmod.title')"
            @update:model-value="modeText = $event"
          />
          <input
            id="file-permissions-mode"
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
          <p v-if="parsedMode === null" class="text-[11px] text-destructive">{{ t("files.properties.invalid") }}</p>
          <p v-if="failure" class="text-[11px] text-destructive">{{ failure.message }}</p>
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
            {{ t("files.chmod.cancel") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="busy || parsedMode === null || !props.entry"
            @click="confirm()"
          >
            {{ t("files.chmod.confirm") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
