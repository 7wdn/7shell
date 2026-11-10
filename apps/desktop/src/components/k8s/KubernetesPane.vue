<script setup lang="ts">
/**
 * The cluster workspace: resources, logs, exec and metrics.
 *
 * This is the contract skeleton: the connection lifecycle and the tree/list views land behind it.
 */
import { ShipWheel } from "@lucide/vue";
import { useI18n } from "vue-i18n";

import type { KubernetesTab } from "@/stores/terminals";

defineProps<{ tab: KubernetesTab; active: boolean }>();

const { t } = useI18n();
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col p-4">
    <header class="flex items-center gap-2">
      <ShipWheel :size="16" class="text-muted-foreground" />
      <h2 class="text-sm font-medium">{{ tab.session.name }}</h2>
    </header>
    <p class="mt-2 text-xs text-muted-foreground">{{ t("k8s.pending") }}</p>
  </section>
</template>
