import { describe, expect, it, vi } from "vitest";

import {
  buildSpecCompletions,
  specCommandCandidates,
  type SpecGeneratorSlot,
  type SpecValueItem,
  type SpecValueSource,
} from "./engine";
import type { SpecCatalog, SpecCommand } from "./types";

const git: SpecCommand = {
  names: ["git"],
  description: "Distributed version control",
  options: [
    { names: ["-C"], description: "Run in a directory", args: [{ name: ["path"], template: "folders" }] },
    { names: ["--version"], description: "Show the version" },
  ],
  subcommands: [
    {
      names: ["checkout", "co"],
      description: "Switch branches",
      options: [
        { names: ["-b"], description: "Create a branch", args: [{ name: ["branch"] }] },
        { names: ["-q", "--quiet"], description: "Quiet" },
      ],
      args: [
        {
          name: ["branch"],
          description: "Branch to switch to",
          isOptional: true,
          generators: [{ script: ["git", "branch", "-a", "--no-color"] }],
          suggestions: [{ name: ["-"], hidden: true }],
        },
        { name: ["pathspec"], isOptional: true, isVariadic: true, template: "filepaths" },
      ],
    },
    {
      names: ["commit"],
      description: "Record changes",
      options: [
        { names: ["-m"], description: "Message", args: [{ name: ["message"] }] },
        { names: ["-a", "--all"], description: "Stage everything" },
      ],
      args: [],
    },
    { names: ["status"], description: "Show the working tree" },
  ],
};

const sudo: SpecCommand = {
  names: ["sudo"],
  description: "Run as another user",
  options: [{ names: ["-u"], description: "User", args: [{ name: ["user"] }] }],
  args: [{ name: ["command"], isCommand: true }],
};

const run: SpecCommand = {
  names: ["run"],
  description: "Run a command",
  args: [{ name: ["command"], isCommand: true }],
};

function catalogOf(...specs: SpecCommand[]): SpecCatalog {
  const registry: Record<string, SpecCommand> = {};
  const byName = new Map<string, SpecCommand>();
  const commandNames: string[] = [];
  for (const spec of specs) {
    registry[spec.names[0]!] = spec;
    for (const name of spec.names) {
      byName.set(name.toLowerCase(), spec);
      commandNames.push(name);
    }
  }
  return { registry, byName, commandNames };
}

const catalog = catalogOf(git, sudo, run);

function build(line: string, prefixLength = 0, values?: SpecValueSource) {
  return buildSpecCompletions({
    line,
    caret: line.length - prefixLength,
    catalog,
    handle: "terminal-1",
    cwd: "/srv/app",
    commandNames: catalog.commandNames,
    values,
  });
}

function labels(line: string, prefixLength = 0, values?: SpecValueSource): string[] {
  return (build(line, prefixLength, values)?.candidates ?? []).map((candidate) => candidate.label);
}

describe("buildSpecCompletions", () => {
  it("offers subcommands for a partly typed first argument", () => {
    expect(labels("git ch")).toContain("checkout");
    expect(labels("git ch")).not.toContain("commit");
  });

  it("offers the aliases when the person is typing an alias", () => {
    const found = build("git co")?.candidates ?? [];
    const alias = found.find((candidate) => candidate.label === "co");
    expect(alias?.detail).toBe("Switch branches");
    expect(found.map((candidate) => candidate.label)).toContain("commit");
  });

  it("descends into a subcommand and offers its options", () => {
    expect(labels("git checkout -")).toEqual(expect.arrayContaining(["-b", "-q"]));
  });

  it("prefers the long spelling once two dashes have been typed", () => {
    const found = labels("git checkout --");
    expect(found).toContain("--quiet");
    expect(found).not.toContain("-q");
  });

  it("stops offering options after the flag terminator", () => {
    const found = labels("git checkout -- ");
    expect(found).not.toContain("--quiet");
  });

  it("knows an option that is waiting for its value", () => {
    const found = labels("git checkout -b ");
    expect(found).toEqual([]);
  });

  it("offers a command word after an argument that takes one", () => {
    const found = labels("run gi");
    expect(found).toContain("git");
  });

  it("marks a path-taking argument as wanting the host's directories", () => {
    expect(build("git checkout main ")?.wantsPath).toBe(true);
  });

  it("returns null for a command the corpus does not know", () => {
    expect(build("unheard-of-command argument")).toBeNull();
  });

  it("returns null inside a comment and inside quotes", () => {
    expect(build("git checkout # comment")).toBeNull();
    expect(build("git 'check")).toBeNull();
  });

  it("keeps a generator's values in the order they were cached and filters them by prefix", () => {
    const warm = vi.fn<(slot: SpecGeneratorSlot) => void>();
    const values: SpecValueItem[] = [{ label: "main" }, { label: "master" }, { label: "maintenance" }];
    const source: SpecValueSource = {
      cached: (key) => (key.length > 0 ? values : null),
      warm,
    };
    const found = labels("git checkout ma", 0, source);
    expect(found).toContain("main");
    expect(found).toContain("master");
    expect(found).toContain("maintenance");
    expect(warm).not.toHaveBeenCalled();
  });

  it("asks for a generator once and offers what it already has while the answer is on its way", () => {
    const warm = vi.fn<(slot: SpecGeneratorSlot) => void>();
    const source: SpecValueSource = {
      cached: () => null,
      warm,
    };
    build("git checkout ", 0, source);
    expect(warm).toHaveBeenCalledTimes(1);
    const slots = warm.mock.calls.map(([slot]) => slot);
    expect(slots.some((slot) => slot.command === "git" && slot.args.includes("branch"))).toBe(true);
    expect(slots.every((slot) => slot.handle === "terminal-1" && slot.cwd === "/srv/app")).toBe(true);
  });

  it("builds the same cache key for every prefix of one token", () => {
    const first = vi.fn<(slot: SpecGeneratorSlot) => void>();
    const second = vi.fn<(slot: SpecGeneratorSlot) => void>();
    build("git checkout m", 0, { cached: () => null, warm: first });
    build("git checkout ma", 0, { cached: () => null, warm: second });
    const key = (calls: typeof first.mock.calls) => calls.find(([slot]) => slot.command === "git")?.[0].key;
    expect(key(first.mock.calls)).toBe(key(second.mock.calls));
  });

  it("offers command names for the command word itself", () => {
    const found = specCommandCandidates(catalog, "gi", 0);
    expect(found.map((candidate) => candidate.label)).toEqual(["git"]);
  });
});
