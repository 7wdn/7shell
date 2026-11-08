import { describe, expect, it } from "vitest";

import { parseValues } from "./completion";

describe("parseValues", () => {
  it("takes the branch marker and the remote prefix off a git branch list", () => {
    const values = parseValues("* main\n  dev\n  remotes/origin/main\n  remotes/origin/HEAD -> origin/main\n", {
      command: "git",
      args: ["branch", "-a"],
      splitOn: null,
    });
    expect(values.map((value) => value.label)).toEqual(["main", "dev", "origin/main"]);
  });

  it("keeps a tab-separated description beside its label", () => {
    const values = parseValues("one\tthe first\ntwo\t\n", {
      command: "example",
      args: [],
      splitOn: null,
    });
    expect(values).toEqual([{ label: "one", detail: "the first" }, { label: "two" }]);
  });

  it("honours a custom separator and drops duplicates", () => {
    const values = parseValues("alpha, beta ,alpha,", {
      command: "example",
      args: [],
      splitOn: ",",
    });
    expect(values.map((value) => value.label)).toEqual(["alpha", "beta"]);
  });

  it("does not treat a generic command's leading spaces as a branch marker", () => {
    const values = parseValues("  keep me\n", { command: "example", args: [], splitOn: null });
    expect(values).toEqual([{ label: "keep me" }]);
  });
});
