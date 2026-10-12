/**
 * Where the columns somebody moved are kept, and what keeps two tables' columns apart.
 *
 * The store is small and this is what is worth asserting about it: that a width belongs to one
 * column of one table, that putting a column back leaves nothing behind, and that a table nobody has
 * touched answers with the same object every time — which is what a table's render depends on.
 */
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it } from "vitest";

import { useColumnsStore } from "@/stores/columns";

beforeEach(() => {
  setActivePinia(createPinia());
});

describe("the widths somebody moved", () => {
  it("belongs to one column of one table", () => {
    const columns = useColumnsStore();

    columns.setWidth("files", "size", 120);
    columns.setWidth("processes", "size", 240);

    expect(columns.movedIn("files").size).toBe(120);
    expect(columns.movedIn("processes").size).toBe(240);
  });

  it("leaves nothing behind when a column is put back", () => {
    const columns = useColumnsStore();
    columns.setWidth("files", "size", 120);
    columns.setWidth("files", "kind", 96);

    columns.resetWidth("files", "size");

    expect(columns.movedIn("files")).toEqual({ kind: 96 });
  });

  it("answers a table nobody has touched with the same empty answer every time", () => {
    const columns = useColumnsStore();

    // Not merely equal: a table that is redrawn reads this on every render, and a fresh object each
    // time would be a render that never settles.
    expect(columns.movedIn("files")).toBe(columns.movedIn("files"));
    expect(columns.movedIn("files")).toEqual({});
  });

  it("keeps the widths of one table when another is written to", () => {
    const columns = useColumnsStore();
    columns.setWidth("files", "size", 120);

    columns.setWidth("processes", "pid", 90);

    expect(columns.movedIn("files").size).toBe(120);
  });
});
