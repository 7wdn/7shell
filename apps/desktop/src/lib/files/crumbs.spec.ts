import { describe, expect, it } from "vitest";

import { firstVisibleCrumb } from "@/lib/files/crumbs";

describe("the breadcrumb run that fits", () => {
  it("keeps every crumb when the path fits", () => {
    expect(firstVisibleCrumb([40, 60, 50], 200)).toBe(0);
  });

  it("folds leading crumbs and keeps the folders nearest the current one", () => {
    expect(firstVisibleCrumb([50, 100, 100], 160)).toBe(2);
  });

  it("keeps the last crumb even when it has to be truncated itself", () => {
    expect(firstVisibleCrumb([300, 400], 120)).toBe(1);
  });

  it("has nothing to choose between when there are no crumbs", () => {
    expect(firstVisibleCrumb([], 200)).toBe(0);
  });
});
