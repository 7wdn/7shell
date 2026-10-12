/**
 * How wide a table's columns have been moved, for as long as the application is running.
 *
 * Nothing is written down, and that is a decision rather than an omission: closing the application
 * puts every column back to the width this build draws it at, which is what the file managers and
 * the database clients this interface is measured against do with theirs. So this is a map and not
 * a setting — the same kind of thing as the transfer list's rows, which also belong to the run
 * rather than to the installation.
 *
 * Keyed by table and then by column, because two tables' columns have nothing to do with each other
 * even when the words match: a file's size and a process's memory are both a number in a column, and
 * an edge moved in one must not move the other.
 *
 * Only the columns somebody has moved are in here. A column that is not is drawn at the width its
 * own table states, which is what makes a change to that default reach everybody rather than only
 * the people who never dragged its edge.
 */
import { defineStore } from "pinia";
import { ref } from "vue";

/**
 * The answer for a table nobody has touched.
 *
 * One object rather than a fresh `{}` per read: what a component draws is worked out from this on
 * every render, and an answer that is a new object each time is a render that repeats itself for
 * ever — the trap `computed` cannot see out of.
 */
const NOTHING_MOVED: Readonly<Record<string, number>> = Object.freeze({});

export const useColumnsStore = defineStore("columns", () => {
  const moved = ref<Record<string, Record<string, number>>>({});

  /** What has been moved in one table: column key to pixels, and nothing at all for a fresh table. */
  function movedIn(table: string): Readonly<Record<string, number>> {
    return moved.value[table] ?? NOTHING_MOVED;
  }

  /**
   * Writes down where a column's edge was put.
   *
   * Called for every pointer move of a drag — a width that is written down once per gesture is a
   * width the column is drawn at only after the pointer is let go, which reads as a drag that does
   * nothing until it is over.
   */
  function setWidth(table: string, column: string, px: number): void {
    moved.value = { ...moved.value, [table]: { ...movedIn(table), [column]: px } };
  }

  /** Puts one column back to the width its table states, which is what double-clicking it does. */
  function resetWidth(table: string, column: string): void {
    const widths = { ...movedIn(table) };
    delete widths[column];
    moved.value = { ...moved.value, [table]: widths };
  }

  return { moved, movedIn, setWidth, resetWidth };
});
