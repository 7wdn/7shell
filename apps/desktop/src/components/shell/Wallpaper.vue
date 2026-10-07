<script setup lang="ts">
/**
 * The picture behind the window, as the window shows it.
 *
 * A component rather than a composable, although it is all effect and no markup to speak of: what it
 * draws is one layer, and the layer's lifetime is the picture's.
 *
 * Three jobs, and the order they are in is the interesting part. The picture is read from the shell
 * whenever the settings name a different one, and turned into something the browser can draw — an
 * object URL, because the bytes arrive as base64 and a data URL of a photograph is a string several
 * megabytes long that the style attribute would carry around. The surfaces in front of it are made
 * translucent while it is there, and put back exactly as they were when it goes.
 *
 * The put-back is why this is a composable with a lifetime rather than a computed style: the window
 * wears a palette, the palette is the stylesheet's business, and a picture coming and going must not
 * leave the window wearing a colour somebody's palette never had.
 */
import { computed, onBeforeUnmount, ref, watch } from "vue";

import { backend } from "@/lib/backend/api";
import { backgroundImageStyle, withTranslucentSurfaces } from "@/lib/settings/background";
import { usePreferencesStore } from "@/stores/preferences";
import { backgroundIsSet } from "@/types/app";

const preferences = usePreferencesStore();

/** What the browser can draw, or nothing while there is no picture — or none that could be read. */
const url = ref<string | null>(null);
let warned = false;

function toObjectUrl(base64: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return URL.createObjectURL(new Blob([bytes]));
}

function forget(): void {
  if (url.value) {
    URL.revokeObjectURL(url.value);
    url.value = null;
  }
}

watch(
  () => preferences.backgroundImage.fileName,
  async (_name, _previous, onCleanup) => {
    let cancelled = false;
    onCleanup(() => {
      cancelled = true;
      forget();
    });

    if (!backgroundIsSet(preferences.backgroundImage)) {
      forget();
      return;
    }

    try {
      const base64 = await backend.background.read();
      if (cancelled) {
        return;
      }
      forget();
      url.value = toObjectUrl(base64);
    } catch (error) {
      if (cancelled) {
        return;
      }
      forget();
      // Said once rather than on every settings write: a picture that cannot be read is a window with
      // no picture, which is a state the window already knows how to be in, and a console full of the
      // same line is how a real problem gets ignored.
      if (!warned) {
        warned = true;
        console.warn("the background picture could not be read, so the window draws none", error);
      }
    }
  },
  { immediate: true },
);

/** What the layer is painted with: the mode, the blur, and the picture itself. */
const style = computed(() => backgroundImageStyle(preferences.backgroundImage, url.value));

/**
 * Puts the surfaces back the way they were, whichever part of the window changed.
 *
 * Re-applied rather than patched when the palette, the theme or the transparency changes: the colours
 * being made translucent are the stylesheet's current ones, and an alpha taken from the previous
 * palette is a window wearing a colour that never existed.
 */
let release: (() => void) | null = null;

function repaint(): void {
  release?.();
  release = null;
  if (!url.value) {
    return;
  }
  release = withTranslucentSurfaces(preferences.backgroundImage.transparency);
}

watch(
  [url, () => preferences.backgroundImage.transparency, () => preferences.wornPalette, () => preferences.resolvedTheme],
  repaint,
  {
    immediate: true,
  },
);

onBeforeUnmount(() => {
  release?.();
  release = null;
  forget();
});

/** Whether anything is drawn at all: the layer is not in the tree without a picture. */
const active = computed(() => url.value !== null);
</script>

<template>
  <!--
    The picture, behind everything and in front of nothing. Fixed rather than absolute so that it does
    not scroll with the pane it happens to be behind, and out of the pointer's way because a background
    is not something to click.
  -->
  <div v-if="active" data-wallpaper class="pointer-events-none fixed inset-0 -z-10 overflow-hidden">
    <div class="h-full w-full" :style="style"></div>
  </div>
</template>
