<script setup lang="ts">
/**
 * Where a panel half sits, drawn as the icon of the position it is in.
 *
 * The button is the previous placement control rather than a chevron: what it shows answers "where
 * is this half?" without opening anything, and pressing it offers the other positions with the same
 * icons. It lives in the half it moves — the file browser's own path row, the commands panel's own
 * header — so moving a half is a control of that half rather than of the terminal strip above it.
 */
import { ExternalLink, PanelBottom, PanelLeft, PanelRight } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { useContextMenu } from "@/composables/useContextMenu";
import { MENU_TITLES } from "@/lib/menu/titles";
import { commandPanelPlacementMenu, panelPlacementMenu } from "@/lib/menu/workspace";
import type { DockedPanelPlacement, TerminalPanelPlacement } from "@/types/app";

const props = defineProps<{
  kind: "files" | "commands";
  placement: TerminalPanelPlacement | DockedPanelPlacement;
}>();

const emit = defineEmits<{
  chooseFiles: [placement: TerminalPanelPlacement];
  chooseCommands: [placement: DockedPanelPlacement];
}>();

const { t } = useI18n();
const { openMenu } = useContextMenu();

const icon = computed(() => {
  if (props.placement === "tab") return ExternalLink;
  if (props.placement === "left") return PanelLeft;
  if (props.placement === "right") return PanelRight;
  return PanelBottom;
});

const label = computed(() =>
  props.kind === "files" ? t("terminal.panel.position") : t("terminal.panel.commandPosition"),
);

function open(event: MouseEvent): void {
  if (props.kind === "files") {
    openMenu(
      event,
      { key: MENU_TITLES.panelPlacement },
      panelPlacementMenu(props.placement as TerminalPanelPlacement, {
        choose: (placement) => emit("chooseFiles", placement),
      }),
    );
    return;
  }
  openMenu(
    event,
    { key: MENU_TITLES.commandPanelPlacement },
    commandPanelPlacementMenu(props.placement as DockedPanelPlacement, {
      choose: (placement) => emit("chooseCommands", placement),
    }),
  );
}
</script>

<template>
  <button
    type="button"
    :data-panel-placement="props.kind === 'files' ? '' : undefined"
    :data-command-placement="props.kind === 'commands' ? '' : undefined"
    :data-placement="props.placement"
    class="grid shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    :title="label"
    :aria-label="label"
    aria-haspopup="true"
    @click.stop="open"
  >
    <component :is="icon" :size="12" />
  </button>
</template>
