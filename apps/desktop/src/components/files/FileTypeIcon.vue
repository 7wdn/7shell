<script setup lang="ts">
/**
 * The drawing beside a name in the file listing.
 *
 * A file that has a drawing of its own wears it at sixteen pixels, which is the size at which a
 * filled picture reads the same as the stroke glyphs around it. Directories wear a solid amber
 * folder, closed in the listing and open while a tree branch is unfolded; like every other file
 * drawing it keeps its colour in every theme. What has no drawing — a generic file, a symbolic
 * link, a device — keeps the stroke glyph, because those are not file types and a coloured picture
 * would claim more than the row knows.
 */
import { File, FileSymlink, HardDrive } from "@lucide/vue";
import { computed } from "vue";

import { fileIconMarkup } from "@/lib/files/icons";
import type { SftpEntry } from "@/types/session";

const props = withDefaults(
  defineProps<{
    readonly name: string;
    readonly kind: SftpEntry["kind"];
    readonly size?: number;
    /** A directory drawn open, which is how a tree shows a branch that is unfolded. */
    readonly open?: boolean;
  }>(),
  { size: 16, open: false },
);

const markup = computed(() => fileIconMarkup(props.name, props.kind));
const box = computed(() => `${props.size}px`);

const fallback = computed(() => {
  switch (props.kind) {
    case "symlink":
      return FileSymlink;
    case "other":
      return HardDrive;
    default:
      return File;
  }
});
</script>

<template>
  <span
    v-if="markup"
    class="file-type-icon inline-flex shrink-0 items-center justify-center"
    :style="{ width: box, height: box }"
    aria-hidden="true"
    v-html="markup"
  />
  <span
    v-else-if="kind === 'directory'"
    class="file-type-icon inline-flex shrink-0 items-center justify-center"
    :style="{ width: box, height: box }"
    aria-hidden="true"
  >
    <svg viewBox="0 0 24 24" fill="#ffca28" focusable="false">
      <path
        v-if="!open"
        d="M3 6a2 2 0 0 1 2-2h3.2a2 2 0 0 1 1.6.8l.8 1.2H19a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"
      />
      <template v-else>
        <path d="M3 6a2 2 0 0 1 2-2h3.2a2 2 0 0 1 1.6.8l.8 1.2H18a2 2 0 0 1 2 2v1H6.5a2 2 0 0 0-1.9 1.37L3 16Z" />
        <path
          d="M5.9 10h15.2a1 1 0 0 1 .95 1.32l-2.3 6.9A2 2 0 0 1 17.8 20H4.2a1 1 0 0 1-.95-1.32l2.3-6.9A1.2 1.2 0 0 1 5.9 10Z"
        />
      </template>
    </svg>
  </span>
  <component :is="fallback" v-else :size="size" class="shrink-0 text-muted-foreground" aria-hidden="true" />
</template>

<style scoped>
/* The copied files carry their own pixel size; the row's box is what decides how big they draw. */
.file-type-icon :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
