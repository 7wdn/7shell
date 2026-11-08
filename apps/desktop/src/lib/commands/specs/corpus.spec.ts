import { readFile } from "node:fs/promises";
import path from "node:path";

import { describe, expect, it, vi } from "vitest";

import { buildSpecCompletions, type SpecGeneratorSlot, type SpecValueSource } from "./engine";
import { catalogFromRaw } from "./load";

const raw = JSON.parse(
  await readFile(path.resolve(process.cwd(), "apps/desktop/src/lib/commands/specs/fig-specs.json"), "utf8"),
);
const catalog = catalogFromRaw(raw);

function candidates(line: string, values?: SpecValueSource) {
  return buildSpecCompletions({
    line,
    caret: line.length,
    catalog,
    handle: "terminal-1",
    cwd: "/srv/app",
    values,
  });
}

function labels(line: string): string[] {
  return (candidates(line)?.candidates ?? []).map((candidate) => candidate.label);
}

describe("the bundled corpus", () => {
  it("knows the commands an operations console is asked for", () => {
    for (const name of ["git", "docker", "kubectl", "systemctl", "journalctl", "npm", "curl", "tar", "apk"]) {
      expect(catalog.byName.has(name), name).toBe(true);
    }
  });

  it("offers git subcommands", () => {
    expect(labels("git ch")).toEqual(expect.arrayContaining(["checkout", "cherry-pick"]));
    expect(labels("git com")).toContain("commit");
  });

  it("offers a subcommand's flags and their long spellings", () => {
    expect(labels("git commit --")).toEqual(expect.arrayContaining(["--message"]));
    expect(labels("git commit -")).toEqual(expect.arrayContaining(["-m"]));
  });

  it("offers docker subcommands and their flags", () => {
    expect(labels("docker ")).toEqual(expect.arrayContaining(["run", "ps"]));
    expect(labels("docker ps --")).toEqual(expect.arrayContaining(["--all"]));
  });

  it("offers kubectl resources after get", () => {
    const found = labels("kubectl get ");
    expect(found.length).toBeGreaterThan(0);
    expect(found.join(" ")).toMatch(/pods|deployments|services/);
  });

  it("offers systemctl verbs", () => {
    expect(labels("systemctl ")).toEqual(expect.arrayContaining(["status", "restart"]));
  });

  it("reads git checkout's branches from a generator, and takes a file list as a path hint", () => {
    const warm = vi.fn<(slot: SpecGeneratorSlot) => void>();
    const result = candidates("git checkout ", { cached: () => null, warm });
    expect(warm.mock.calls.some(([slot]) => slot.command === "git" && slot.args.includes("branch"))).toBe(true);
    expect(result?.wantsPath).toBe(true);
  });

  it("offers cached branch values for a partial word", () => {
    const values = [{ label: "main" }, { label: "release" }];
    const result = candidates("git checkout ma", {
      cached: () => values,
      warm: () => undefined,
    });
    expect(result?.candidates.map((candidate) => candidate.label)).toContain("main");
  });
});
