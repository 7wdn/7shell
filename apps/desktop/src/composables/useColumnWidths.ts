/**
 * A table's columns, bound to the widths somebody has moved and to the room the line has.
 *
 * This is the piece two tables share. It hands the template the custom properties to draw with, the
 * least width the line may be drawn at, and one call per heading that answers everything an edge
 * needs. Which columns a table has, and what its line spends around them, is each table's own
 * business and is handed in.
 *
 * The room is measured and then watched, rather than assumed: the window, the sidebar's divider and
 * the file panel's divider all change it without touching the columns, and it is not only the
 * ceiling of something — it is also the room the columns nobody has moved divide between them. A
 * share of a width that no longer holds is a column drawn at the wrong size.
 *
 * Nothing here writes anything down. Where a width ends up is `stores/columns.ts`, which forgets it
 * when the application closes.
 */
import { type Ref, computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import {
  type ColumnLine,
  type ColumnWidth,
  drawnWidths,
  floorOf,
  layoutOf,
  lineMinWidth,
  movedTo,
  widthVars,
} from "@/lib/columns";
import { useColumnsStore } from "@/stores/columns";

export interface ColumnWidthsRequest {
  /** The table these columns belong to, which is what keeps its widths its own. */
  readonly table: string;
  readonly columns: readonly ColumnWidth[];
  /** What the line spends on something other than its columns. */
  readonly line: ColumnLine;
  /** The element the columns are laid out in: their line, and the thing its width is read from. */
  readonly element: Ref<HTMLElement | null>;
}

export function useColumnWidths({ table, columns, line, element }: ColumnWidthsRequest) {
  const { t } = useI18n();
  const store = useColumnsStore();

  /** How wide the line is drawn, in pixels. Nought until it has been laid out once. */
  const lineWidth = ref(0);
  let observer: ResizeObserver | null = null;

  const layout = computed(() => layoutOf(columns, store.movedIn(table), line));
  /** What each column is drawn at, with the panel's spare room already divided. */
  const drawn = computed(() => drawnWidths(layout.value, lineWidth.value));
  /** The widths, as the custom properties a heading and the rows under it both read. */
  const vars = computed(() => widthVars(layout.value, lineWidth.value));
  /** The width the line has to be drawn at least: below it the line scrolls rather than squeezes. */
  const minWidth = computed(() => lineMinWidth(layout.value));

  /**
   * Reads the room the columns have, which is the width of the line they are drawn on.
   *
   * A line that has not been laid out yet reports nothing, and nought is what is kept then: no spare
   * room is divided, and every column is drawn at the width it asks for, which is true rather than
   * invented.
   */
  function measure(): void {
    lineWidth.value = element.value?.clientWidth ?? 0;
  }

  onMounted(() => {
    // Once for the frame it mounted into, and again after the styles it arrived with have been
    // applied: a measurement taken before the padding is known is a measurement of another width.
    void nextTick(measure);
    const box = element.value;
    if (box && typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(() => measure());
      observer.observe(box);
    }
  });

  onBeforeUnmount(() => {
    observer?.disconnect();
    observer = null;
  });

  /**
   * Everything one column's edge needs: where it is now, how narrow it may be drawn, and what it is
   * called for somebody who cannot see it. The label is built from the heading the column shows, so
   * the two cannot come to name different columns.
   *
   * There is no widest, and so nothing here says there is one: the strip scrolls, which is the
   * answer to how wide a column may be.
   */
  function edge(key: string, heading: string) {
    return {
      width: drawn.value[key] ?? 0,
      min: floorOf(layout.value, key),
      label: t("common.resizeColumn", { column: heading }),
    };
  }

  /** Where a drag puts the edge: the columns' own rule keeps it off the floor, and the store keeps it. */
  function move(key: string, px: number): void {
    store.setWidth(table, key, movedTo(layout.value, key, px));
  }

  /** Puts the column back to the width its table states: nothing has been moved for it. */
  function reset(key: string): void {
    store.resetWidth(table, key);
  }

  return { vars, minWidth, edge, move, reset };
}
