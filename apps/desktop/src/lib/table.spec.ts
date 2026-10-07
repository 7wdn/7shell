/**
 * The table vocabulary, which is what every table in the interface wears.
 *
 * These are the rules from the module's own comment, written as assertions: they are the parts a
 * reader would notice going wrong long before a screenshot did — a row that stops answering the
 * pointer, a selected row that the pointer erases, a panel row that grows a rule it was asked not to
 * have. What each *component* does with the vocabulary is checked in that component's own spec.
 */
import { describe, expect, it } from "vitest";

import { TABLE, rowClass } from "@/lib/table";

describe("the table vocabulary", () => {
  it("marks the row under the pointer on both surfaces, pressed or not", () => {
    for (const surface of ["page", "panel"] as const) {
      expect(rowClass(surface), `${surface}: the band is what says which row is being read`).toContain(
        "hover:bg-accent/50",
      );
      expect(rowClass(surface, { clickable: true })).toContain("hover:bg-accent/50");
    }
  });

  it("keeps a selected row marked under the pointer rather than replaced by it", () => {
    const picked = rowClass("page", { selected: true });

    expect(picked).toContain("bg-accent/60");
    expect(picked, "the pointer deepens a selected row: two states, two shades").toContain("hover:bg-accent/70");
    expect(rowClass("page"), "and a row that is not selected is not drawn as one").not.toContain("bg-accent/60");
  });

  it("gives the pointer and the keyboard to the rows that can be pressed, and neither to the rest", () => {
    const pressable = rowClass("page", { clickable: true });

    expect(pressable).toContain("cursor-pointer");
    expect(pressable).toContain("focus-visible:bg-accent/60");
    expect(rowClass("page"), "a row that is only read is not offered as a control").not.toContain("cursor-pointer");
  });

  it("rules the pages' rows apart, and leaves the panel's rows to the band alone", () => {
    expect(TABLE.page.row).toContain("border-b");
    expect(TABLE.panel.row, "at 10px a hairline under every reading is more furniture than it is worth").toBe("");
    expect(rowClass("page")).toContain("border-b border-border/50");
    expect(rowClass("panel")).not.toContain("border-b");
  });

  it("pins the pages' headings over the rows and lets the panel's go", () => {
    expect(TABLE.page.head).toContain("sticky");
    expect(TABLE.page.head, "a row scrolled under a translucent strip is read through it").toContain("bg-background");
    expect(TABLE.panel.head).toBe("");
  });
});
