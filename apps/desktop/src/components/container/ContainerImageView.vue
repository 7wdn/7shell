<script setup lang="ts">
/**
 * The images on one engine, the containers they run, and the transfers that move them.
 *
 * The related-containers column is a link rather than a number: "three containers use this" is only
 * useful as "which three", so clicking it hands the image to the container list as a filter. That
 * is why this view emits instead of filtering itself.
 *
 * Pulls and pushes are rows in a panel below the table rather than modal progress dialogs: several
 * can run at once, and a dialog that blocks the table would make starting the second one impossible.
 *
 * Loading an archive from a file is deliberately absent: the wire takes a grant produced by a native
 * dialog, and this build has no such command to ask for one. An entry that could not produce a token
 * would be a button that always failed.
 */
import { ArrowDownToLine, ArrowUpFromLine, Ellipsis, LogIn, Play, RotateCw, Tag, Trash2 } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import type { MenuEntry } from "@/lib/menu/types";
import { TABLE, rowClass } from "@/lib/table";
import { cn } from "@/lib/utils";
import {
  type ContainerSession,
  filterImages,
  imageRef,
  relatedContainers,
  shortId,
  supportsTag,
  useContainerStore,
} from "@/stores/container";
import type { ContainerTab } from "@/stores/terminals";
import type { ContainerImage, ContainerImageTransfer } from "@/types/container";

import ContainerCreateDialog from "./ContainerCreateDialog.vue";
import ContainerDialog from "./ContainerDialog.vue";
import ImageDetailDrawer from "./ImageDetailDrawer.vue";
import ImageTaskPanel from "./ImageTaskPanel.vue";
import ImageTransferDialog from "./ImageTransferDialog.vue";
import RegistryLoginDialog from "./RegistryLoginDialog.vue";

const props = defineProps<{ tab: ContainerTab; session: ContainerSession }>();
const emit = defineEmits<{ "view-related": [reference: string] }>();

const { t } = useI18n();
const containers = useContainerStore();
const { openMenu } = useContextMenu();

const text = ref("");
const rows = computed(() => filterImages(props.session.images, text.value));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

// -- dialogs -------------------------------------------------------------------

const transferOpen = ref(false);
const transferMode = ref<"pull" | "push">("pull");
const transferImage = ref("");
const loginOpen = ref(false);
const detailOpen = ref(false);
const detailTarget = ref<ContainerImage | null>(null);
const creating = ref(false);
const createImage = ref("");
const tagging = ref<ContainerImage | null>(null);
const tagText = ref("");
const tagProblem = ref<string | null>(null);
const tagBusy = ref(false);
const removing = ref<ContainerImage | null>(null);
const pruning = ref(false);

function openTransfer(mode: "pull" | "push", image = ""): void {
  transferMode.value = mode;
  transferImage.value = image;
  transferOpen.value = true;
}

function startTransfer(request: ContainerImageTransfer): void {
  const starting =
    transferMode.value === "pull"
      ? containers.pullImage(props.tab.id, request)
      : containers.pushImage(props.tab.id, request);
  // The task panel owns the outcome from here; a refusal to start is drawn on its row.
  void starting;
}

function openDetail(image: ContainerImage): void {
  detailTarget.value = image;
  detailOpen.value = true;
}

function closeDetail(): void {
  detailOpen.value = false;
  detailTarget.value = null;
}

function runImage(image: ContainerImage): void {
  createImage.value = imageRef(image);
  creating.value = true;
}

// -- tag ------------------------------------------------------------------------

function startTag(image: ContainerImage): void {
  tagging.value = image;
  tagText.value = imageRef(image);
  tagProblem.value = null;
}

async function submitTag(): Promise<void> {
  const target = tagging.value;
  const value = tagText.value.trim();
  if (!target || tagBusy.value) {
    return;
  }
  if (value === "") {
    tagProblem.value = t("container.image.tagRequired");
    return;
  }
  tagBusy.value = true;
  try {
    await containers.tagImage(props.tab.id, imageRef(target), value);
    tagging.value = null;
  } catch (error) {
    tagProblem.value = describeShellError(translate, asShellError(error, "container_image_tag")).message;
  } finally {
    tagBusy.value = false;
  }
}

function closeTag(): void {
  tagging.value = null;
}

// -- removal -------------------------------------------------------------------

async function confirmRemove(): Promise<void> {
  const target = removing.value;
  removing.value = null;
  if (!target) {
    return;
  }
  try {
    await containers.removeImage(props.tab.id, imageRef(target));
  } catch {
    // The store kept the envelope; the pane's banner is where it is shown.
  }
}

async function confirmPrune(): Promise<void> {
  pruning.value = false;
  try {
    await containers.pruneImages(props.tab.id);
  } catch {
    // Same: a failed prune leaves the list as it was and the banner says why.
  }
}

// -- the row's menu -------------------------------------------------------------

function openRowMenu(event: MouseEvent, image: ContainerImage): void {
  const reference = imageRef(image);
  const entries: MenuEntry[] = [
    {
      kind: "action",
      id: "run",
      labelKey: "container.images.run",
      icon: Play,
      params: { name: reference },
      run: () => runImage(image),
    },
  ];

  if (supportsTag(props.session.runtime)) {
    entries.push({
      kind: "action",
      id: "tag",
      labelKey: "container.images.tag",
      icon: Tag,
      params: { name: reference },
      run: () => startTag(image),
    });
  }

  entries.push(
    {
      kind: "action",
      id: "push",
      labelKey: "container.image.push",
      icon: ArrowUpFromLine,
      run: () => openTransfer("push", reference),
    },
    {
      kind: "action",
      id: "remove",
      labelKey: "container.image.remove",
      icon: Trash2,
      danger: true,
      run: () => {
        removing.value = image;
      },
    },
  );

  openMenu(event, { key: "container.image.menu", params: { name: reference } }, entries);
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2.5 py-1.5">
      <input
        v-model="text"
        type="text"
        spellcheck="false"
        class="h-6 w-56 rounded-md border border-border bg-background px-2 text-[11px] focus:outline-none"
        :placeholder="t('container.images.filter')"
        :aria-label="t('container.images.filter')"
      />
      <div class="min-w-0 flex-1"></div>

      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
        @click="openTransfer('pull')"
      >
        <ArrowDownToLine :size="11" />
        {{ t("container.image.pull") }}
      </button>
      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
        @click="loginOpen = true"
      >
        <LogIn :size="11" />
        {{ t("container.registry.short") }}
      </button>
      <button
        type="button"
        class="rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
        @click="pruning = true"
      >
        {{ t("container.image.prune") }}
      </button>
      <button
        type="button"
        class="grid size-6 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('container.refresh')"
        :aria-label="t('container.refresh')"
        @click="
          containers.refresh(props.tab.id);
          containers.loadImages(props.tab.id);
        "
      >
        <RotateCw :size="12" />
      </button>
    </div>

    <div class="min-h-0 flex-1 overflow-auto">
      <table
        :class="cn(TABLE.page.table, 'table-fixed')"
        :style="{ minWidth: '780px' }"
        :aria-label="t('container.rail.images')"
      >
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="w-[24%]" :class="TABLE.page.headCell">{{ t("container.col.repository") }}</th>
            <th class="w-[13%]" :class="TABLE.page.headCell">{{ t("container.col.tag") }}</th>
            <th class="w-[11%]" :class="TABLE.page.headCell">{{ t("container.col.id") }}</th>
            <th class="w-[9%]" :class="TABLE.page.headCell">{{ t("container.col.size") }}</th>
            <th class="w-[15%]" :class="TABLE.page.headCell">{{ t("container.col.created") }}</th>
            <th class="w-[9%]" :class="TABLE.page.headCell">{{ t("container.col.related") }}</th>
            <th class="w-[7rem]" :class="TABLE.page.headCell">{{ t("container.col.actions") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="image in rows"
            :key="`${image.id}:${image.repository}:${image.tag}`"
            :class="cn(rowClass('page', { clickable: true }))"
            @click="openDetail(image)"
          >
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="image.repository">{{ image.repository }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="image.tag">{{ image.tag }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]" :title="image.id">{{ shortId(image.id) }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate font-mono text-[11px]">{{ image.size }}</span>
            </td>
            <td :class="TABLE.page.cell">
              <span class="block truncate text-[11px] text-muted-foreground" :title="image.createdAt">
                {{ image.createdAt }}
              </span>
            </td>
            <td :class="TABLE.page.cell" @click.stop>
              <button
                v-if="relatedContainers(props.session.containers, image) > 0"
                type="button"
                class="rounded-sm px-1 text-[11px] text-primary transition-colors hover:bg-accent"
                :title="t('container.images.related', { name: imageRef(image) })"
                @click="emit('view-related', imageRef(image))"
              >
                {{ relatedContainers(props.session.containers, image) }}
              </button>
            </td>
            <td :class="TABLE.page.cell" @click.stop>
              <span class="flex items-center gap-0.5">
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.image.run')"
                  :aria-label="t('container.image.run')"
                  @click="runImage(image)"
                >
                  <Play :size="12" />
                </button>
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.image.push')"
                  :aria-label="t('container.image.push')"
                  @click="openTransfer('push', imageRef(image))"
                >
                  <ArrowUpFromLine :size="12" />
                </button>
                <button
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :title="t('container.image.more', { name: imageRef(image) })"
                  :aria-label="t('container.image.more', { name: imageRef(image) })"
                  @click="openRowMenu($event, image)"
                >
                  <Ellipsis :size="12" />
                </button>
              </span>
            </td>
          </tr>
          <tr v-if="rows.length === 0">
            <td :colspan="7" :class="cn(TABLE.page.cell, TABLE.page.emptyCell)">
              {{ t("container.images.none") }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <ImageTaskPanel v-if="props.session.tasks.length > 0" :tab-id="props.tab.id" :tasks="props.session.tasks" />

    <ImageDetailDrawer
      :mode="detailOpen ? 'detail' : null"
      :tab-id="props.tab.id"
      :target="detailTarget"
      @close="closeDetail()"
    />

    <ContainerCreateDialog
      :open="creating"
      :tab-id="props.tab.id"
      :runtime="props.session.runtime"
      :initial-image="createImage"
      @update:open="creating = $event"
    />

    <ImageTransferDialog
      :open="transferOpen"
      :mode="transferMode"
      :runtime="props.session.runtime"
      :initial-image="transferImage"
      @update:open="transferOpen = $event"
      @start="startTransfer"
    />

    <RegistryLoginDialog
      :open="loginOpen"
      :tab-id="props.tab.id"
      :runtime="props.session.runtime"
      @update:open="loginOpen = $event"
    />

    <ContainerDialog
      :open="tagging !== null"
      :title="t('container.image.tagTitle')"
      :description="t('container.image.tagMessage', { name: tagging ? imageRef(tagging) : '' })"
      :busy="tagBusy"
      @update:open="closeTag"
    >
      <input
        v-model="tagText"
        type="text"
        spellcheck="false"
        class="w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
        :aria-label="t('container.image.tagTitle')"
        @keydown.enter.prevent="submitTag()"
      />
      <p v-if="tagProblem" class="mt-2 text-[11px] text-destructive">{{ tagProblem }}</p>
      <template #footer>
        <button
          type="button"
          class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
          @click="closeTag()"
        >
          {{ t("common.action.cancel") }}
        </button>
        <button
          type="button"
          class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="tagBusy"
          @click="submitTag()"
        >
          {{ t("container.image.tagVerb") }}
        </button>
      </template>
    </ContainerDialog>

    <ConfirmDialog
      :open="removing !== null"
      :title="t('container.image.removeTitle', { name: removing ? imageRef(removing) : '' })"
      :message="t('container.image.removeMessage')"
      :confirm-label="t('container.image.removeVerb')"
      danger
      @confirm="confirmRemove()"
      @update:open="removing = null"
    />

    <ConfirmDialog
      :open="pruning"
      :title="t('container.image.prune')"
      :message="t('container.image.pruneMessage')"
      :confirm-label="t('container.image.prune')"
      danger
      @confirm="confirmPrune()"
      @update:open="pruning = $event"
    />
  </div>
</template>
