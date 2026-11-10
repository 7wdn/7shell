<script setup lang="ts">
/**
 * The container workspace: containers, images and the runtime behind them.
 *
 * This is the contract skeleton: the connection lifecycle and the three views land behind it. It is
 * kept mounted while hidden for the same reason every other pane is — a log stream or a transfer
 * panel must survive a look at another tab.
 */
import { Boxes } from "@lucide/vue";
import { useI18n } from "vue-i18n";

import type { ContainerTab } from "@/stores/terminals";

defineProps<{ tab: ContainerTab; active: boolean }>();

const { t } = useI18n();
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col p-4">
    <header class="flex items-center gap-2">
      <Boxes :size="16" class="text-muted-foreground" />
      <h2 class="text-sm font-medium">{{ tab.session.name }}</h2>
    </header>
    <p class="mt-2 text-xs text-muted-foreground">{{ t("container.pending") }}</p>
  </section>
</template>
