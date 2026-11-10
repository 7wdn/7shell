<script setup lang="ts">
/**
 * One image: what it was built from, how it is configured, and its raw inspect.
 *
 * The layers are read first because they are the thing a size question is really about — "why is
 * this image 1.2 GB" is answered by the layer whose command dwarfs the rest. A layer's command is
 * one line until it is clicked, since a `RUN` with a long script is otherwise a wall of text over
 * every other layer.
 *
 * The raw inspect stays available as JSON: the fields the interface draws are the common ones, and
 * an image configured in a way this view does not know about is still readable there.
 */
import { X } from "@lucide/vue";
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import { shortId, useContainerStore } from "@/stores/container";
import type { ContainerImage, ImageLayer } from "@/types/container";

const props = defineProps<{
  mode: "detail" | null;
  tabId: string;
  target: ContainerImage | null;
}>();

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const containers = useContainerStore();

const tab = ref<"layers" | "detail" | "json">("layers");
const layers = ref<ImageLayer[]>([]);
const layersFailure = ref<string | null>(null);
const layersLoading = ref(false);
const raw = ref("");
const detailFailure = ref<string | null>(null);
const detailLoading = ref(false);
const expanded = ref<ReadonlySet<number>>(new Set());
const copied = ref(false);
let layersGeneration = 0;
let detailGeneration = 0;
let copyTimer: number | undefined;

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function fullRef(image: ContainerImage): string {
  return `${image.repository}:${image.tag}`;
}

/**
 * What the inspect and history calls are given.
 *
 * The full `repo:tag` when there is one — `nerdctl` refuses a short id with "multiple ids found" —
 * and the id otherwise, for an image whose tags have all been removed.
 */
function queryRef(image: ContainerImage): string {
  if (image.repository !== "<none>" && image.tag && image.tag !== "<none>") {
    return fullRef(image);
  }
  return image.id;
}

const title = computed(() => (props.target ? fullRef(props.target) : ""));

const prettyRaw = computed(() => {
  if (raw.value === "") {
    return "";
  }
  try {
    const parsed = JSON.parse(raw.value);
    return JSON.stringify(Array.isArray(parsed) ? parsed[0] : parsed, null, 2);
  } catch {
    return raw.value;
  }
});

function toggleLayer(index: number): void {
  const next = new Set(expanded.value);
  if (next.has(index)) {
    next.delete(index);
  } else {
    next.add(index);
  }
  expanded.value = next;
}

async function loadLayers(): Promise<void> {
  const target = props.target;
  if (!target) {
    return;
  }
  const generation = ++layersGeneration;
  layers.value = [];
  layersFailure.value = null;
  layersLoading.value = true;
  try {
    const answer = await containers.imageHistory(props.tabId, queryRef(target));
    if (generation === layersGeneration) {
      layers.value = answer;
    }
  } catch (error) {
    if (generation === layersGeneration) {
      layersFailure.value = describeShellError(translate, asShellError(error, "container_image_history")).message;
    }
  } finally {
    if (generation === layersGeneration) {
      layersLoading.value = false;
    }
  }
}

async function loadDetail(): Promise<void> {
  const target = props.target;
  if (!target) {
    return;
  }
  const generation = ++detailGeneration;
  raw.value = "";
  detailFailure.value = null;
  detailLoading.value = true;
  try {
    const answer = await containers.imageInspect(props.tabId, queryRef(target));
    if (generation === detailGeneration) {
      raw.value = answer ?? "";
    }
  } catch (error) {
    if (generation === detailGeneration) {
      detailFailure.value = describeShellError(translate, asShellError(error, "container_image_inspect")).message;
    }
  } finally {
    if (generation === detailGeneration) {
      detailLoading.value = false;
    }
  }
}

/**
 * The first object of the inspect answer.
 *
 * A runtime may answer with an array (Docker) or an object (some container tools); the fields are
 * read from the raw JSON because the image's structure is not the container's and there is no wire
 * type that flattens it.
 */
interface InspectShape {
  readonly RepoTags?: unknown;
  readonly Created?: unknown;
  readonly Size?: unknown;
  readonly Architecture?: unknown;
  readonly Os?: unknown;
  readonly Config?: {
    readonly Entrypoint?: unknown;
    readonly Cmd?: unknown;
    readonly WorkingDir?: unknown;
    readonly User?: unknown;
    readonly ExposedPorts?: unknown;
    readonly Volumes?: unknown;
    readonly Env?: unknown;
    readonly Labels?: unknown;
  };
}

function firstObject(): InspectShape | null {
  if (raw.value === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw.value);
    const first = Array.isArray(parsed) ? parsed[0] : parsed;
    return first && typeof first === "object" ? (first as InspectShape) : null;
  } catch {
    return null;
  }
}

function joinList(value: unknown): string {
  return Array.isArray(value) ? value.filter(Boolean).join(", ") : "";
}

function joinKeys(value: unknown): string {
  return value && typeof value === "object" ? Object.keys(value).join(", ") : "";
}

function joinEntries(value: unknown): string {
  if (!value || typeof value !== "object") {
    return "";
  }
  return Object.entries(value as Record<string, unknown>)
    .map(([key, entry]) => `${key}=${String(entry)}`)
    .join("\n");
}

function humanSize(bytes: unknown): string {
  const amount = Number(bytes);
  if (!Number.isFinite(amount) || amount <= 0) {
    return "";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = amount;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit] ?? "B"}`;
}

interface DetailSection {
  label: string;
  fields: [string, string][];
}

const detailSections = computed<DetailSection[]>(() => {
  const image = firstObject();
  if (!image) {
    return [];
  }
  const config = image.Config;
  const sections: DetailSection[] = [
    {
      label: t("container.detail.sectionOverview"),
      fields: [
        [t("container.imageFields.repoTags"), joinList(image.RepoTags)],
        [t("container.detail.fieldId"), props.target ? shortId(props.target.id) : ""],
        [t("container.imageFields.size"), humanSize(image.Size)],
        [
          t("container.imageFields.created"),
          image.Created ? String(image.Created).replace("T", " ").replace("Z", " UTC") : "",
        ],
      ],
    },
    {
      label: t("container.imageFields.sectionConfig"),
      fields: [
        [t("container.overview.arch"), [image.Architecture, image.Os].filter(Boolean).join("/")],
        [t("container.detail.fieldEntrypoint"), joinList(config?.Entrypoint)],
        [t("container.detail.fieldCommand"), joinList(config?.Cmd)],
        [t("container.detail.fieldWorkDir"), config?.WorkingDir ? String(config.WorkingDir) : ""],
        [t("container.detail.fieldUser"), config?.User ? String(config.User) : ""],
        [t("container.imageFields.exposedPorts"), joinKeys(config?.ExposedPorts)],
        [t("container.imageFields.volumes"), joinKeys(config?.Volumes)],
      ],
    },
    {
      label: t("container.detail.sectionEnv"),
      fields: joinList(config?.Env) !== "" ? [[t("container.detail.sectionEnv"), joinList(config?.Env)]] : [],
    },
    {
      label: t("container.imageFields.labels"),
      fields:
        joinEntries(config?.Labels) !== "" ? [[t("container.imageFields.labels"), joinEntries(config?.Labels)]] : [],
    },
  ];
  return sections.filter((section) => section.fields.length > 0);
});

async function copyRaw(): Promise<void> {
  try {
    await navigator.clipboard.writeText(prettyRaw.value);
    copied.value = true;
    window.clearTimeout(copyTimer);
    copyTimer = window.setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    // The JSON is selectable on screen; the clipboard is only the shortcut.
  }
}

watch(
  () => [props.mode, props.target?.id],
  () => {
    if (props.mode === "detail" && props.target) {
      tab.value = "layers";
      void loadLayers();
      void loadDetail();
    } else {
      layers.value = [];
      raw.value = "";
      layersFailure.value = null;
      detailFailure.value = null;
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  window.clearTimeout(copyTimer);
});
</script>

<template>
  <div v-if="props.mode !== null" class="absolute inset-0 z-20" @click.self="emit('close')">
    <div
      class="absolute inset-y-0 right-0 flex w-[min(40rem,100%)] flex-col border-l border-border bg-card shadow-xl"
      role="dialog"
      :aria-label="title"
    >
      <header class="flex shrink-0 items-center justify-between gap-2 border-b border-border px-3 py-2">
        <span class="min-w-0 truncate font-mono text-xs" :title="title">{{ title }}</span>
        <button
          type="button"
          class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('common.action.close')"
          :aria-label="t('common.action.close')"
          @click="emit('close')"
        >
          <X :size="12" />
        </button>
      </header>

      <div class="flex shrink-0 gap-0.5 border-b border-border px-2">
        <button
          type="button"
          class="border-b-2 px-2 py-1 text-[11px] transition-colors"
          :class="
            tab === 'layers'
              ? 'border-primary text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="tab = 'layers'"
        >
          {{ t("container.image.layers") }}
        </button>
        <button
          type="button"
          class="border-b-2 px-2 py-1 text-[11px] transition-colors"
          :class="
            tab === 'detail'
              ? 'border-primary text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="tab = 'detail'"
        >
          {{ t("container.image.detail") }}
        </button>
        <button
          type="button"
          class="border-b-2 px-2 py-1 text-[11px] transition-colors"
          :class="
            tab === 'json'
              ? 'border-primary text-foreground'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="tab = 'json'"
        >
          JSON
        </button>
      </div>

      <div v-show="tab === 'layers'" class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
        <p v-if="layersLoading" class="text-[11px] text-muted-foreground">{{ t("container.detail.loading") }}</p>
        <p v-else-if="layersFailure" class="text-[11px] text-destructive select-text">{{ layersFailure }}</p>
        <p v-else-if="layers.length === 0" class="text-[11px] text-muted-foreground">—</p>
        <div
          v-for="(layer, index) in layers"
          :key="index"
          class="flex items-baseline gap-2 border-b border-border/50 py-1 last:border-b-0"
        >
          <span class="w-8 shrink-0 text-right font-mono text-[10px] text-muted-foreground">
            #{{ layers.length - index }}
          </span>
          <button
            type="button"
            class="min-w-0 flex-1 text-left font-mono text-[11px] select-text"
            :class="expanded.has(index) ? 'break-all whitespace-pre-wrap' : 'truncate'"
            :title="layer.createdBy || layer.id"
            @click="toggleLayer(index)"
          >
            {{ layer.createdBy || shortId(layer.id) || "—" }}
          </button>
          <span class="flex shrink-0 items-baseline gap-2">
            <span v-if="layer.size" class="font-mono text-[10px] text-muted-foreground">{{ layer.size }}</span>
            <span v-if="layer.createdAt" class="text-[10px] text-muted-foreground">{{ layer.createdAt }}</span>
          </span>
        </div>
      </div>

      <div v-show="tab === 'detail'" class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
        <p v-if="detailLoading" class="text-[11px] text-muted-foreground">{{ t("container.detail.loading") }}</p>
        <p v-else-if="detailFailure" class="text-[11px] text-destructive select-text">{{ detailFailure }}</p>
        <section v-for="section in detailSections" :key="section.label" class="mb-3">
          <h3 class="mb-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            {{ section.label }}
          </h3>
          <dl>
            <div
              v-for="field in section.fields"
              :key="field[0]"
              class="flex gap-3 border-b border-border/50 py-1 last:border-b-0"
            >
              <dt class="w-32 shrink-0 text-[10px] text-muted-foreground">{{ field[0] }}</dt>
              <dd class="min-w-0 flex-1 font-mono text-[11px] break-all whitespace-pre-wrap select-text">
                {{ field[1] || "—" }}
              </dd>
            </div>
          </dl>
        </section>
      </div>

      <div v-show="tab === 'json'" class="flex min-h-0 flex-1 flex-col">
        <div class="shrink-0 border-b border-border px-3 py-1.5">
          <button
            type="button"
            class="rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-40"
            :disabled="prettyRaw === ''"
            @click="copyRaw()"
          >
            {{ copied ? t("container.detail.copied") : t("container.detail.copy") }}
          </button>
        </div>
        <pre class="min-h-0 flex-1 overflow-auto px-3 py-2 font-mono text-[11px] whitespace-pre-wrap select-text">{{
          prettyRaw
        }}</pre>
      </div>
    </div>
  </div>
</template>
