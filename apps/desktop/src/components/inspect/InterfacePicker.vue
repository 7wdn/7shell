<script setup lang="ts">
/**
 * The interface picker: which of the host's interfaces the figures are about.
 *
 * It is a control and not a label because a machine has more than one, and which one matters
 * depends on what the person is doing — a bond carries the traffic, a bridge carries the
 * containers, and loopback is a real answer to "did anything reach the local service".
 *
 * The busiest interface is the first choice rather than a prompt: it is what the panel shows when
 * nobody has chosen, a reading that follows the traffic rather than one that has to be told where to
 * look. Its value is the empty string, which is also what the monitor records for "no particular
 * interface", so choosing it and never having chosen are the same state rather than two.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
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

const options = computed<SelectOption[]>(() => [
  { value: "", label: t("inspect.network.automatic") },
  ...interfaces.value.map((name) => ({ value: name, label: name })),
]);

function choose(value: string): void {
  monitor.selectInterface(value === "" ? null : value);
}
</script>

<template>
  <Select
    :model-value="chosen"
    :options="options"
    size="compact"
    :disabled="interfaces.length === 0"
    :aria-label="t('inspect.network.pick')"
    trigger-class="max-w-40 font-mono"
    @update:model-value="choose"
  />
</template>
