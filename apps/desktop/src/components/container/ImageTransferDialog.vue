<script setup lang="ts">
/**
 * Pull or push one image, with only the options that change what travels.
 *
 * `platform` and `allTags` are pull settings; a push sends what the local tag names and nothing
 * else. Skipping TLS verification is offered only on the runtimes whose CLI takes it per command —
 * on Docker it is a daemon setting, so a checkbox here would be a switch that does nothing.
 */
import { computed, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { supportsInsecure } from "@/stores/container";
import type { ContainerImageTransfer, ContainerRuntime } from "@/types/container";

import ContainerDialog from "./ContainerDialog.vue";

const props = defineProps<{
  open: boolean;
  mode: "pull" | "push";
  runtime: ContainerRuntime;
  initialImage?: string;
}>();

const emit = defineEmits<{ "update:open": [value: boolean]; start: [request: ContainerImageTransfer] }>();

const { t } = useI18n();

const form = reactive({ image: "", platform: "", insecure: false, allTags: false });
const imageProblem = ref<string | null>(null);

const pull = computed(() => props.mode === "pull");
const canSkipTls = computed(() => supportsInsecure(props.runtime));

/** The platforms worth offering; the list accepts anything typed as well. */
const platformOptions = computed(() => [
  { value: "", label: t("container.transfer.platformDefault") },
  ...["linux/amd64", "linux/arm64", "linux/arm/v7", "linux/386", "linux/ppc64le", "linux/s390x"].map((value) => ({
    value,
    label: value,
  })),
]);

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    form.image = props.initialImage ?? "";
    form.platform = "";
    // Off, and it stays somebody's decision: verification is what makes a registry trustworthy,
    // and a checkbox that arrives pre-ticked is one nobody reads.
    form.insecure = false;
    form.allTags = false;
    imageProblem.value = null;
  },
);

function submit(): void {
  const image = form.image.trim();
  if (image === "") {
    imageProblem.value = t("container.transfer.imageRequired");
    return;
  }

  emit("start", {
    image,
    platform: pull.value && form.platform.trim() !== "" ? form.platform.trim() : undefined,
    allTags: pull.value && form.allTags ? true : undefined,
    insecure: canSkipTls.value && form.insecure ? true : undefined,
  });
  emit("update:open", false);
}
</script>

<template>
  <ContainerDialog
    :open="props.open"
    :title="pull ? t('container.transfer.pullTitle') : t('container.transfer.pushTitle')"
    @update:open="emit('update:open', $event)"
  >
    <div class="flex flex-col gap-2.5">
      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.transfer.image") }}</span>
        <input
          v-model="form.image"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
          :class="imageProblem ? 'border-destructive' : ''"
          :placeholder="pull ? 'nginx:latest' : 'registry.example.com/repo/name:tag'"
          @input="imageProblem = null"
          @keydown.enter.prevent="submit()"
        />
        <span v-if="imageProblem" class="text-[10px] text-destructive">{{ imageProblem }}</span>
      </label>

      <template v-if="pull">
        <div class="flex flex-col gap-1">
          <span class="text-[11px] text-muted-foreground">{{ t("container.transfer.platform") }}</span>
          <Select v-model="form.platform" :options="platformOptions" :aria-label="t('container.transfer.platform')" />
        </div>
        <label class="flex items-center gap-1.5 text-[11px]">
          <input v-model="form.allTags" type="checkbox" class="size-3 accent-[var(--primary)]" />
          {{ t("container.transfer.allTags") }}
        </label>
      </template>

      <label v-if="canSkipTls" class="flex items-center gap-1.5 text-[11px]">
        <input v-model="form.insecure" type="checkbox" class="size-3 accent-[var(--primary)]" />
        {{ t("container.transfer.skipTlsVerify") }}
      </label>
    </div>

    <template #footer>
      <button
        type="button"
        class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
        @click="emit('update:open', false)"
      >
        {{ t("common.action.cancel") }}
      </button>
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
        @click="submit()"
      >
        {{ pull ? t("container.transfer.submit") : t("container.transfer.pushSubmit") }}
      </button>
    </template>
  </ContainerDialog>
</template>
