<script setup lang="ts">
/**
 * The interface picker: which of the host's interfaces the figures are about.
 *
 * It is a control and not a label because a machine has more than one, and which one matters
 * depends on what the person is doing — a bond carries the traffic, a bridge carries the
 * containers, and loopback is a real answer to "did anything reach the local service".
 *
 * The empty choice is "whichever is busiest", which is what the panel shows when nobody has
 * chosen: a reading that follows the traffic rather than one that has to be told where to look.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { useMonitorStore } from "@/stores/monitor";

const props = defineProps<{
  /**
   * Whether the sample on screen is about this picker's host.
   *
   * The panel is always about the host it is drawn for, and passes `true`; a page passes what its
   * frame worked out. Without it, a page behind another tab would offer the interfaces of whichever
   * machine happens to be on screen.
   */
  live: boolean;
}>();

const { t } = useI18n();
const monitor = useMonitorStore();

/** The interfaces on offer: the host's own, and none at all when this is not its sample. */
const interfaces = computed(() => (props.live ? monitor.interfaces : []));

/** What the picker shows: the interface the figures are about, or nothing when there is none. */
const chosen = computed(() => (props.live ? (monitor.interfaceName ?? "") : ""));

function choose(event: Event): void {
  const value = (event.target as HTMLSelectElement).value;
  monitor.selectInterface(value === "" ? null : value);
}
</script>

<template>
  <select
    class="max-w-40 rounded-md border border-border bg-background px-1.5 py-0.5 font-mono text-[11px] text-foreground transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
    :value="chosen"
    :disabled="interfaces.length === 0"
    :aria-label="t('inspect.network.pick')"
    :title="t('inspect.network.pick')"
    @change="choose"
  >
    <option value="">{{ t("inspect.network.automatic") }}</option>
    <option v-for="name in interfaces" :key="name" :value="name">{{ name }}</option>
  </select>
</template>
