<script setup lang="ts">
/**
 * The product mark: the application icon, the size of a title bar.
 *
 * `art/icon.svg` is the artwork, and this is that file's own geometry mapped onto a 32-unit box by
 * the plate it sits on — the same 30.5/928 the coordinates below were worked out with, plus the
 * 0.75 the plate is inset by — so the mark in the title bar and the icon in the taskbar are one
 * drawing at two sizes. Change them together.
 *
 * It is drawn in the icon's own colours rather than the design tokens, and that is deliberate: a
 * mark that took its colour from the theme would be a different picture from the one the taskbar
 * and the installer show, and it would be a grey one, because the palettes' accent for text is not
 * this blue. The icon carries its own dark plate, which is what lets one picture read on a light
 * title bar and a dark one without knowing which it is on.
 */
withDefaults(defineProps<{ size?: number }>(), { size: 18 });
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 32 32" fill="none" role="img" aria-label="7Shell" class="shrink-0">
    <rect x="0.75" y="0.75" width="30.5" height="30.5" rx="6.84" fill="#12161d" stroke="#2b3342" stroke-width="0.13" />
    <path
      d="M9.16 8.9 L21 8.9 L12.98 23.1"
      stroke="#4da3ff"
      stroke-width="3.29"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <path d="M17.91 23.1 L22.84 23.1" stroke="#e6ebf5" stroke-width="3.29" stroke-linecap="round" />
  </svg>
</template>
