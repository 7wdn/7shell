<script setup lang="ts">
/**
 * The product mark: a prompt, a chevron followed by an underscore.
 *
 * Drawn as SVG rather than shipped as a bitmap so it stays crisp at any size and takes its
 * colour from the design tokens. `art/icon.png` is the same shape for places that need a file.
 */
withDefaults(defineProps<{ size?: number }>(), { size: 18 });
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 32 32" fill="none" role="img" aria-label="7Shell" class="shrink-0">
    <rect
      x="0.75"
      y="0.75"
      width="30.5"
      height="30.5"
      rx="7.5"
      class="fill-primary/10 stroke-border"
      stroke-width="1.5"
    />
    <path
      d="M11 10.5 L17.5 16 L11 21.5"
      class="stroke-primary"
      stroke-width="2.4"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <rect x="19.5" y="19.6" width="6.5" height="2.6" rx="1.3" class="fill-foreground/70" />
  </svg>
</template>
