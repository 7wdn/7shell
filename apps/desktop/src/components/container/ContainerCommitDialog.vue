<script setup lang="ts">
/**
 * Turns a container's file system into an image.
 *
 * It is deliberately a dialog and not a menu entry that just runs: a commit takes as long as writing
 * the layers and cannot be taken back, so it says what it is about to write before it starts. The
 * container itself is untouched — only a new image appears — which is what the hint says.
 *
 * Only the Docker CLI is offered this; see `supportsCommit` in the store, which is what keeps the
 * menu entry off the other runtimes.
 */
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import { useContainerStore } from "@/stores/container";
import type { ContainerSummary } from "@/types/container";

import ContainerDialog from "./ContainerDialog.vue";

const props = defineProps<{
  open: boolean;
  tabId: string;
  target: ContainerSummary | null;
}>();

const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const containers = useContainerStore();

const image = ref("");
const author = ref("");
const message = ref("");
const busy = ref(false);
const failure = ref<string | null>(null);
const imageProblem = ref<string | null>(null);

const targetName = computed(() => props.target?.name ?? "");

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    image.value = "";
    author.value = "";
    message.value = "";
    busy.value = false;
    failure.value = null;
    imageProblem.value = null;
  },
);

async function submit(): Promise<void> {
  const target = props.target;
  if (!target || busy.value) {
    return;
  }
  const reference = image.value.trim();
  if (reference === "") {
    imageProblem.value = t("container.commit.imageRequired");
    return;
  }

  busy.value = true;
  failure.value = null;
  try {
    await containers.commit(props.tabId, {
      containerId: target.id,
      image: reference,
      author: author.value.trim() === "" ? undefined : author.value.trim(),
      message: message.value.trim() === "" ? undefined : message.value.trim(),
    });
    emit("update:open", false);
  } catch (error) {
    failure.value = describeShellError(translate, asShellError(error, "container_image_commit")).message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <ContainerDialog
    :open="props.open"
    :title="t('container.commit.title')"
    :description="t('container.commit.hint')"
    :busy="busy"
    @update:open="emit('update:open', $event)"
  >
    <div class="flex flex-col gap-2.5">
      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.commit.container") }}</span>
        <span class="font-mono text-xs break-all">{{ targetName }}</span>
      </div>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.commit.image") }}</span>
        <input
          v-model="image"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
          :class="imageProblem ? 'border-destructive' : ''"
          :placeholder="t('container.commit.imagePlaceholder')"
          @input="imageProblem = null"
        />
        <span v-if="imageProblem" class="text-[10px] text-destructive">{{ imageProblem }}</span>
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.commit.author") }}</span>
        <input
          v-model="author"
          type="text"
          class="rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none"
        />
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.commit.message") }}</span>
        <textarea
          v-model="message"
          rows="2"
          class="resize-none rounded-md border border-border bg-background px-2 py-1 text-xs focus:outline-none"
        ></textarea>
      </label>

      <p v-if="busy" class="text-[11px] text-muted-foreground">{{ t("container.commit.running") }}</p>
      <p v-if="failure" class="text-[11px] text-destructive select-text">{{ failure }}</p>
    </div>

    <template #footer>
      <button
        type="button"
        class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-40"
        :disabled="busy"
        @click="emit('update:open', false)"
      >
        {{ t("common.action.cancel") }}
      </button>
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
        :disabled="busy"
        @click="submit()"
      >
        {{ t("container.commit.submit") }}
      </button>
    </template>
  </ContainerDialog>
</template>
