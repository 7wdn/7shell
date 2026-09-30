<script setup lang="ts">
/**
 * Minimize, maximize/restore and close, drawn by the interface because the window has no system
 * title bar on Windows and Linux.
 *
 * Not rendered on macOS, where the traffic lights are the platform's own.
 */
import { Copy, Minus, Square, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";

defineProps<{
  isMaximized: boolean;
  minimize: () => void;
  toggleMaximize: () => void;
  close: () => void;
}>();

const { t } = useI18n();
</script>

<template>
  <div class="flex items-stretch">
    <button
      type="button"
      class="grid w-12 place-items-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      :title="t('shell.window.minimize')"
      :aria-label="t('shell.window.minimize')"
      @click="minimize()"
    >
      <Minus :size="14" />
    </button>

    <button
      type="button"
      class="grid w-12 place-items-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      :title="isMaximized ? t('shell.window.restore') : t('shell.window.maximize')"
      :aria-label="isMaximized ? t('shell.window.restore') : t('shell.window.maximize')"
      @click="toggleMaximize()"
    >
      <Copy v-if="isMaximized" :size="12" />
      <Square v-else :size="12" />
    </button>

    <button
      type="button"
      class="grid w-12 place-items-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground"
      :title="t('shell.window.close')"
      :aria-label="t('shell.window.close')"
      @click="close()"
    >
      <X :size="15" />
    </button>
  </div>
</template>
