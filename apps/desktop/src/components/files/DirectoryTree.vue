<script setup lang="ts">
/**
 * The remote directory tree.
 *
 * Children are fetched when a branch is opened and then kept, so closing and reopening one costs
 * nothing. The tree follows the listing rather than driving it: wherever the right hand side
 * navigates, its ancestors open here, which is what keeps the two panes from drifting apart.
 *
 * A change anywhere makes every cached branch suspect — a folder created in one place changes what
 * a folder elsewhere holds — so the whole cache is dropped and the open branches re-read. That is
 * one round trip per open branch, which is a price worth paying for never showing a stale tree.
 */
import { ChevronRight, Folder, FolderOpen, LoaderCircle } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { ancestorsOf } from "@/lib/files/format";
import { useFilesStore } from "@/stores/files";

const props = defineProps<{
  handle: string;
  /** Where the listing is; the tree follows it rather than the other way round. */
  currentPath: string;
  /**
   * How wide to draw it, in pixels.
   *
   * Handed in rather than written here, because the width is the panel's: it is whatever the divider
   * beside this pane was last left at, and a width this component decided would be one the divider
   * could not move.
   */
  width: number;
}>();

const emit = defineEmits<{ navigate: [path: string] }>();

const { t } = useI18n();
const files = useFilesStore();

/** What one row is, flattened for rendering: the walk decides which rows exist at all. */
interface Row {
  path: string;
  name: string;
  depth: number;
  expanded: boolean;
  loading: boolean;
}

const expanded = ref<Set<string>>(new Set(["/"]));
/** Branches that have been read. Holding an entry — even an empty one — is what marks it read. */
const branches = ref<Record<string, readonly { path: string; name: string }[]>>({});
const loading = ref<Set<string>>(new Set());

/** The rows to draw, walking only into branches that are open. */
const rows = computed<Row[]>(() => {
  const flattened: Row[] = [];

  const walk = (path: string, name: string, depth: number): void => {
    const isExpanded = expanded.value.has(path);
    flattened.push({
      path,
      name,
      depth,
      expanded: isExpanded,
      loading: loading.value.has(path),
    });

    if (!isExpanded) {
      return;
    }
    for (const child of branches.value[path] ?? []) {
      walk(child.path, child.name, depth + 1);
    }
  };

  walk("/", t("files.tree.root"), 0);
  return flattened;
});

/** Reads anything that is open and has never been read. */
async function readMissing(): Promise<void> {
  const wanted = [...expanded.value].filter((path) => !(path in branches.value) && !loading.value.has(path));
  await Promise.all(wanted.map((path) => read(path)));
}

async function read(path: string): Promise<void> {
  loading.value = new Set(loading.value).add(path);
  try {
    const directories = await files.directories(props.handle, path);
    branches.value = {
      ...branches.value,
      [path]: directories.map((entry) => ({ path: entry.path, name: entry.name })),
    };
  } finally {
    const next = new Set(loading.value);
    next.delete(path);
    loading.value = next;
  }
}

function toggle(row: Row): void {
  const next = new Set(expanded.value);
  if (row.expanded) {
    next.delete(row.path);
  } else {
    next.add(row.path);
  }
  expanded.value = next;
  void readMissing();
}

// The listing moved: open exactly the ancestors that make the new place visible, and check that
// every open branch has been read at least once.
watch(
  () => props.currentPath,
  (path) => {
    expanded.value = new Set([...expanded.value, ...ancestorsOf(path)]);
    void readMissing();
  },
  { immediate: true },
);

// Something changed somewhere: nothing remembered can be trusted.
watch(
  () => files.revision,
  () => {
    branches.value = {};
    void readMissing();
  },
);

function depthPadding(depth: number): string {
  return `${8 + depth * 12}px`;
}

/** The name shown for a row: the root has no last segment to show. */
function label(row: Row): string {
  return row.path === "/" ? t("files.tree.root") : row.name;
}

/** True when a row is the directory being listed, or a parent of it. */
function onTheWay(path: string): boolean {
  return props.currentPath === path || props.currentPath.startsWith(`${path}/`);
}
</script>

<template>
  <!--
    The tree is a list of places to click, not text to select: without this the pointer turns into an
    I-beam over every name, and a drag picks up letters instead of rows.

    No border of its own: the line at its right edge is the divider's, which is the same line whoever
    drags it — two lines a pixel apart would be a border that thickened when the pointer was nowhere
    near it.
  -->
  <nav
    class="shrink-0 cursor-default overflow-auto py-1 select-none"
    :style="{ width: `${props.width}px` }"
    :aria-label="t('files.tree.title')"
  >
    <ul>
      <li v-for="row in rows" :key="row.path">
        <div
          class="flex items-center gap-1 pr-2 text-sm"
          :class="props.currentPath === row.path ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/50'"
        >
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center text-muted-foreground disabled:opacity-30"
            :style="{ marginLeft: depthPadding(row.depth) }"
            :disabled="row.loading"
            :aria-expanded="row.expanded"
            :aria-label="row.expanded ? t('files.tree.collapse') : t('files.tree.expand')"
            @click="toggle(row)"
          >
            <LoaderCircle v-if="row.loading" :size="12" class="animate-spin" />
            <ChevronRight v-else :size="12" :class="row.expanded ? 'rotate-90' : ''" />
          </button>

          <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left"
            :title="row.path"
            @click="emit('navigate', row.path)"
          >
            <FolderOpen v-if="row.expanded" :size="13" class="shrink-0 text-primary" />
            <Folder
              v-else
              :size="13"
              class="shrink-0"
              :class="onTheWay(row.path) ? 'text-primary' : 'text-muted-foreground'"
            />
            <span class="truncate">{{ label(row) }}</span>
          </button>
        </div>
      </li>
    </ul>
  </nav>
</template>
