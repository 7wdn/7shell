import { describe, expect, it } from "vitest";

import type { CatalogEntry } from "@/lib/commands/catalog";
import {
  buildCommandCompletions,
  type CompletionCandidate,
  type CompletionSources,
  type PathSource,
} from "@/lib/commands/completion";
import { subcommandsOf } from "@/lib/commands/subcommands";
import type { CommandHistoryEntry, Snippet } from "@/types/snippet";
import type { SftpEntry } from "@/types/session";

function saved(name: string, command: string, slotHistory: readonly (readonly string[])[] = []): Snippet {
  return { id: name, groupId: "default", name, command, appendCr: true, order: 0, slotHistory };
}

function ran(command: string, at = 1_000): CommandHistoryEntry {
  return { command, at };
}

function command(name: string): CatalogEntry {
  return { name, kind: "command" };
}

function host(name: string, kind: SftpEntry["kind"] = "file"): SftpEntry {
  return { name, path: `/somewhere/${name}`, kind, size: 1 };
}

/** A source that answers from a fixed listing, and remembers what it was asked about. */
function directories(byDirectory: Readonly<Record<string, readonly SftpEntry[]>>, asked: string[] = []) {
  return {
    asked,
    source: (cwd: string | null): PathSource => ({
      cwd,
      entries: (directory) => {
        asked.push(directory);
        return byDirectory[directory] ?? null;
      },
    }),
  };
}

function build(
  line: string,
  sources: Partial<CompletionSources>,
  describe: Readonly<Record<string, string>> = {},
  paths?: PathSource,
  acceptance?: (insert: string) => number,
): readonly CompletionCandidate[] {
  return buildCommandCompletions({
    line,
    caret: line.length,
    sources: { saved: [], history: [], catalog: [], ...sources },
    describe: (name) => describe[name] ?? null,
    subcommands: subcommandsOf,
    describeSubcommand: (command, word) => describe[`${command} ${word}`] ?? null,
    kindLabel: (kind) => kind,
    paths,
    acceptance,
  });
}

describe("what the menu offers", () => {
  it("lands on the exact command rather than on a saved line that starts the same way", () => {
    const candidates = build("git", {
      saved: [saved("状态", "git status")],
      catalog: [command("git"), command("gitk")],
    });

    expect(candidates[0]?.label).toBe("git");
    expect(candidates[0]?.kind).toBe("command");
  });

  it("puts what somebody made above what we ship when both answer equally well", () => {
    const candidates = build("gr", {
      saved: [saved("日志", "grep -n error app.log")],
      history: [ran("grep --color=auto -i todo")],
      catalog: [command("grep"), command("grepdiff")],
    });

    expect(candidates.map((candidate) => candidate.kind)).toEqual(["saved", "history", "command", "command"]);
  });

  it("fills a saved line in with the values it was last sent with", () => {
    const candidates = build("res", {
      saved: [saved("重启服务", "systemctl restart #1", [["nginx"]])],
    });

    expect(candidates[0]).toMatchObject({ insert: "systemctl restart nginx", detail: "systemctl restart #1" });
  });

  it("leaves a slot the command has never been sent with alone", () => {
    const candidates = build("res", {
      saved: [saved("重启服务", "systemctl restart #1 #2", [["nginx"]])],
    });

    expect(candidates[0]?.insert).toBe("systemctl restart nginx #2");
  });

  it("offers a line once, from the better of two sources that have it", () => {
    const line = "tail -f /var/log/syslog";
    const candidates = build("tail", {
      saved: [saved("跟随日志", line)],
      history: [ran(line)],
    });

    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({ kind: "saved", insert: line });
  });

  it("does not offer a whole line somebody has already typed out in full", () => {
    const candidates = build("uptime", {
      saved: [saved("看开机时长", "uptime")],
      history: [ran("uptime")],
      catalog: [command("uptime"), command("uptimed")],
    });

    // The whole-line sources would insert exactly what is there — nothing to complete. The catalogue
    // row stays: taking it leaves a space after the word, and its one line is worth reading.
    expect(candidates.map((candidate) => `${candidate.kind}:${candidate.insert}`)).toEqual([
      "command:uptime",
      "command:uptimed",
    ]);
  });

  it("offers nothing where a shell word is not a command name", () => {
    const sources = { catalog: [command("var"), command("log")] };

    expect(build("ls -l /va", sources), "an argument").toEqual([]);
    expect(build("ls # note", sources), "a comment").toEqual([]);
    expect(build('cd "/va', sources), "inside quotes").toEqual([]);
  });

  it("lists what is saved and what was run for an empty word, and not the catalogue", () => {
    const candidates = build("", {
      saved: [saved("重启服务", "systemctl restart #1")],
      history: [ran("uptime")],
      catalog: [command("ls")],
    });

    expect(candidates.map((candidate) => candidate.insert)).toEqual(["systemctl restart #1", "uptime"]);
  });

  it("says which characters answered, in the label or in the description", () => {
    const byName = build("gr", { catalog: [command("grep")] });
    expect(byName[0]?.labelRanges).toEqual([{ start: 0, end: 2 }]);

    const byCommand = build("err", { saved: [saved("日志", "grep error app.log")] });
    expect(byCommand[0]).toMatchObject({
      labelRanges: [],
      detailRanges: [{ start: 5, end: 8 }],
    });
  });

  it("says what the candidate replaces, so only the word being typed is", () => {
    const candidates = build("sudo gre", { catalog: [command("grep")] });

    expect(candidates[0]).toMatchObject({ from: 5, insert: "grep" });
  });

  it("says what kind of thing a candidate is when it has nothing to say about it", () => {
    expect(build("ls", { catalog: [command("ls")] })[0]?.detail).toBe("command");
    expect(build("ls", { catalog: [command("ls")] }, { ls: "List the entries of a directory" })[0]?.detail).toBe(
      "List the entries of a directory",
    );
  });

  it("offers nothing at all for a word nothing answers", () => {
    expect(build("xyzzy", { catalog: [command("ls")], saved: [saved("重启", "systemctl restart")] })).toEqual([]);
  });
});

describe("the word after a command", () => {
  it("offers the table's words for the first argument, in the order the table has them", () => {
    const candidates = build("systemctl ", {});

    expect(candidates.slice(0, 3).map((candidate) => candidate.label)).toEqual(["status", "start", "stop"]);
    expect(candidates.every((candidate) => candidate.kind === "subcommand")).toBe(true);
    expect(candidates[0]).toMatchObject({ insert: "status", wholeLine: false, from: 10 });
  });

  it("narrows the table with what was typed, and still ranks a prefix above a match inside a word", () => {
    const labels = build("git ch", {}).map((candidate) => candidate.label);

    expect(labels[0]).toBe("checkout");
    // `fetch` and `branch` answer `ch` too — inside the word rather than at its start — and are
    // offered under it rather than withheld: the list is read from the top.
    expect(labels).toContain("fetch");
    expect(labels.length).toBeGreaterThan(1);
  });

  it("says what kind of word it is where this build has nothing to say about it", () => {
    expect(build("systemctl daemon", {})[0]?.detail).toBe("subcommand");
    expect(build("systemctl rest", {}, { "systemctl restart": "Restart a unit" })[0]?.detail).toBe("Restart a unit");
  });

  it("has nothing for a command the table does not know", () => {
    expect(build("some-own-script ", {})).toEqual([]);
  });

  it("stops leaning on the table past the first argument, and offers what somebody typed there", () => {
    // `git checkout` wants a branch, not another word from git's own list.
    expect(build("git checkout ", {})).toEqual([]);

    const candidates = build("git checkout ma", { history: [ran("git checkout main")] });
    expect(candidates.map((candidate) => `${candidate.kind}:${candidate.insert}`)).toEqual(["history:main"]);
  });

  it("offers what somebody typed after this command before, in the same position", () => {
    const candidates = build("systemctl ", {
      history: [ran("systemctl restart nginx"), ran("systemctl status nginx")],
    });

    // The table leads — it is what we ship, and it is what is usually wanted — and the history adds
    // the words the table does not have in this position. `nginx` is the second argument of a line
    // somebody ran, and belongs to the position after this one.
    expect(candidates.map((candidate) => candidate.insert)).toEqual([
      "status",
      "start",
      "stop",
      "restart",
      "reload",
      "enable",
      "disable",
      "daemon-reload",
      "list-units",
      "is-active",
      "show",
    ]);
  });

  it("offers the history's word in the position it was typed in, past the table's reach", () => {
    const candidates = build("systemctl restart ", { history: [ran("systemctl restart nginx")] });

    expect(candidates.map((candidate) => `${candidate.kind}:${candidate.insert}`)).toEqual(["history:nginx"]);
  });

  it("skips a history line that reaches the command through a wrapper", () => {
    // Guessing where the command sits in a line somebody's shell wrote is how a wrong word gets in.
    expect(build("systemctl ", { history: [ran("sudo systemctl restart nginx")] }).map((c) => c.insert)).not.toContain(
      "nginx",
    );
  });

  it("answers nothing for a word that is already this one", () => {
    expect(build("systemctl status", { history: [ran("systemctl status")] })).toHaveLength(1);
    expect(build("systemctl status", { history: [ran("systemctl status")] })[0]).toMatchObject({
      kind: "subcommand",
      insert: "status",
    });
  });
});

describe("a path on the host", () => {
  const root = { "/": [host("etc", "directory"), host("tmp", "directory"), host("var", "directory")] };

  it("offers what the host has in the directory the token points at", () => {
    const paths = directories(root);
    const candidates = build("cd /va", {}, {}, paths.source(null));

    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({
      kind: "path",
      label: "var/",
      insert: "/var/",
      detail: "path",
      from: 3,
      wholeLine: false,
    });
  });

  it("asks the host about the directory and nothing else", () => {
    const paths = directories({});
    build("cd /etc/ho", {}, {}, paths.source(null));

    expect(paths.asked).toEqual(["/etc"]);
  });

  it("resolves a bare name against where the shell is, for a command that takes one", () => {
    const paths = directories({ "/var/log": [host("logs.1"), host("logs.2.gz")] });
    const candidates = build("cat lo", {}, {}, paths.source("/var/log"));

    expect(candidates.map((candidate) => `${candidate.kind}:${candidate.insert}`)).toEqual([
      "path:logs.1",
      "path:logs.2.gz",
    ]);
  });

  it("does not read a directory for a bare word that is not a path", () => {
    const paths = directories({ "/root": [host("notes.txt")] });
    build("uptime not", {}, {}, paths.source("/root"));

    expect(paths.asked).toEqual([]);
  });

  it("reads for a token that can only be a path, whatever command it follows", () => {
    const paths = directories({ "/": [host("tmp", "directory")] });
    const candidates = build("some-own-script /tm", {}, {}, paths.source("/root"));

    expect(paths.asked).toEqual(["/"]);
    expect(candidates[0]).toMatchObject({ label: "tmp/", insert: "/tmp/" });
  });

  it("says nothing while a directory has not been read yet", () => {
    expect(build("cd /va", {}, {}, directories({}).source(null))).toEqual([]);
  });

  it("offers the table's words before the host's, since the words are what is usually wanted", () => {
    const paths = directories({ "/": [host("mnt", "directory")] });
    const candidates = build("ls ", {}, {}, paths.source("/"));

    // The table has no word starting with `m`, so this is the flag list and the path together.
    expect(candidates.map((candidate) => candidate.kind).slice(0, 2)).toEqual(["subcommand", "subcommand"]);
    expect(candidates.map((candidate) => candidate.insert)).toContain("mnt/");
  });

  it("works without a host to ask, for an absolute path", () => {
    // No channel: the source has no cwd and cannot read anything, so there is nothing to offer —
    // and asking for a relative path is a question with no answer rather than an accident.
    const candidates = build("cd /va", {}, {}, { cwd: null, entries: () => null });

    expect(candidates).toEqual([]);
  });
});

describe("what somebody has taken before", () => {
  it("is put above a row they have not, when both answer equally", () => {
    const candidates = build("gr", { catalog: [command("grep"), command("grok")] }, {}, undefined, (insert) =>
      insert === "grok" ? 200 : 0,
    );

    expect(candidates.map((candidate) => candidate.insert)).toEqual(["grok", "grep"]);
  });

  it("cannot outrank a better match", () => {
    // The first candidate answers the prefix and the second only answers inside the word: a hint is
    // worth less than that difference, which is what keeps the list showing what somebody typed.
    const candidates = build("gr", { catalog: [command("grep"), command("algrep")] }, {}, undefined, (insert) =>
      insert === "algrep" ? 300 : 0,
    );

    expect(candidates[0]?.insert).toBe("grep");
  });
});
