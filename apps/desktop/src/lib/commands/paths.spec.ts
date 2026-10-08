import { describe, expect, it } from "vitest";

import {
  completionTextFor,
  entryLabel,
  entryMatches,
  looksLikePath,
  MAX_PATH_CANDIDATES,
  pathQueryAt,
  takesPaths,
  type PathQuery,
} from "@/lib/commands/paths";
import type { SftpEntry } from "@/types/session";

function entry(name: string, kind: SftpEntry["kind"] = "file"): SftpEntry {
  return { name, path: `/somewhere/${name}`, kind, size: 1 };
}

/** A token that is expected to parse, so the assertions below read as the case they are about. */
function query(token: string, cwd: string | null): PathQuery {
  const parsed = pathQueryAt(token, cwd);
  if (parsed === null) {
    throw new Error(`${JSON.stringify(token)} should have parsed`);
  }
  return parsed;
}

describe("what a token points at", () => {
  it("reads an absolute path without being told where the shell is", () => {
    expect(pathQueryAt("/va", null)).toMatchObject({ directory: "/", name: "va" });
    expect(pathQueryAt("/etc/ho", "/home/labuser")).toMatchObject({ directory: "/etc", name: "ho" });
    expect(pathQueryAt("/", null)).toMatchObject({ directory: "/", name: "" });
    expect(pathQueryAt("/var/log/", "/root")).toMatchObject({ directory: "/var/log", name: "" });
  });

  it("resolves a relative one against where the shell is", () => {
    expect(pathQueryAt("logs", "/var")).toMatchObject({ directory: "/var", name: "logs" });
    expect(pathQueryAt("./run.sh", "/srv/app")).toMatchObject({ directory: "/srv/app", name: "run.sh" });
    expect(pathQueryAt("../etc", "/var/log")).toMatchObject({ directory: "/var", name: "etc" });
    expect(pathQueryAt("a/b/c", "/srv")).toMatchObject({ directory: "/srv/a/b", name: "c" });
  });

  it("answers nothing for a relative path with no known directory", () => {
    expect(pathQueryAt("logs", null)).toBeNull();
    expect(pathQueryAt("./run.sh", null)).toBeNull();
    expect(pathQueryAt("", null)).toBeNull();
  });

  it("treats an empty token as the directory itself", () => {
    expect(pathQueryAt("", "/var/log")).toMatchObject({ directory: "/var/log", name: "" });
  });
});

describe("what an entry completes to", () => {
  it("keeps the directory part of the token and rewrites the last segment", () => {
    expect(completionTextFor(query("/va", null), "var/")).toBe("/var/");
    expect(completionTextFor(query("../lo", "/var/log"), "logs.moved/")).toBe("../logs.moved/");
    expect(completionTextFor(query("", "/etc"), "hosts")).toBe("hosts");
    expect(completionTextFor(query("/etc/", null), "hosts")).toBe("/etc/hosts");
  });

  it("says a directory is one the way a shell does", () => {
    expect(entryLabel(entry("tmp", "directory"))).toBe("tmp/");
    expect(entryLabel(entry("hosts"))).toBe("hosts");
    expect(entryLabel(entry("link", "symlink"))).toBe("link");
  });
});

describe("which entries answer", () => {
  const entries = [entry(".ssh", "directory"), entry("logs", "directory"), entry(".bash_history")];

  it("narrows by the segment being typed, in the order the host listed them", () => {
    expect(entryMatches(entries, query("/", null)).map((match) => match.label)).toEqual([
      ".ssh/",
      "logs/",
      ".bash_history",
    ]);
    expect(entryMatches(entries, query("/lo", null)).map((match) => match.label)).toEqual(["logs/"]);
  });

  it("cuts a directory nobody would read, after the ones that answer", () => {
    const many = Array.from({ length: 200 }, (_, index) => entry(`file-${index}`));

    expect(entryMatches(many, query("/file", null))).toHaveLength(MAX_PATH_CANDIDATES);
    expect(entryMatches(many, query("/file-199", null))).toHaveLength(1);
  });

  it("says nothing for entries that do not answer at all", () => {
    expect(entryMatches(entries, query("/xyz", null))).toEqual([]);
  });
});

describe("when a directory is worth reading", () => {
  it("knows the commands whose first word is usually a path", () => {
    expect(takesPaths("cd")).toBe(true);
    expect(takesPaths("Cat")).toBe(true);
    expect(takesPaths("uptime")).toBe(false);
  });

  it("knows a token that can only be a path", () => {
    expect(looksLikePath("/etc")).toBe(true);
    expect(looksLikePath("./run.sh")).toBe(true);
    expect(looksLikePath("../logs")).toBe(true);
    expect(looksLikePath("uptime")).toBe(false);
  });
});
