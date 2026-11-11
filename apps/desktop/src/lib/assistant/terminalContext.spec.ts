/**
 * The privacy promise of the context snapshot, held by examples rather than by intention.
 *
 * Portions of this spec are derived from MIT-licensed code; see THIRD_PARTY_NOTICES.md.
 * Copyright (c) 2026 jinpy.he — released under the MIT License, reproduced in that file.
 *
 * Every case here is a shape somebody could lose a credential to, or a shape that would arrive at a
 * model as noise. The redaction set is conservative on purpose, so the last group is as important as
 * the first: ordinary output must come through untouched, or the assistant is looking at a garbled
 * screen.
 */
import { describe, expect, it } from "vitest";

import {
  AI_OUTPUT_MAX_CHARS,
  AI_OUTPUT_MAX_LINES,
  countLines,
  prepareAiOutputSnapshot,
  prepareTerminalContext,
  redactTerminalOutput,
  stripAnsiEscapes,
  truncateTail,
} from "@/lib/assistant/terminalContext";

describe("truncateTail", () => {
  it("keeps the last lines when there are more than the ceiling", () => {
    const text = Array.from({ length: 300 }, (_, index) => `line-${index}`).join("\n");
    const out = truncateTail(text);

    expect(out.split("\n")).toHaveLength(AI_OUTPUT_MAX_LINES);
    expect(out).toContain("line-299");
    expect(out).not.toContain("line-0\n");
  });

  it("keeps the last characters when one line is longer than the ceiling", () => {
    expect(truncateTail("x".repeat(40_000)).length).toBeLessThanOrEqual(AI_OUTPUT_MAX_CHARS);
    expect(truncateTail("ab".repeat(AI_OUTPUT_MAX_CHARS))).toHaveLength(AI_OUTPUT_MAX_CHARS);
  });

  it("returns an empty string unchanged", () => {
    expect(truncateTail("")).toBe("");
  });
});

describe("redactTerminalOutput", () => {
  it("blanks the value of an assignment and keeps the name", () => {
    expect(redactTerminalOutput("export DB_PASSWORD=hunter2")).toBe("export DB_PASSWORD=***");
    expect(redactTerminalOutput('password: "s3cret"')).toBe("password: ***");
    expect(redactTerminalOutput("api_key=abc123def456")).toBe("api_key=***");
    expect(redactTerminalOutput("clientSecret: very-quiet")).toBe("clientSecret: ***");
  });

  it("replaces a standalone token whole", () => {
    expect(redactTerminalOutput("Authorization: Bearer eyJhbGci.abc.def")).toBe("Authorization: ***");
    expect(redactTerminalOutput("AKIAIOSFODNN7EXAMPLE")).toBe("***");
    expect(redactTerminalOutput("token: ghp_" + "a".repeat(36))).toBe("token: ***");
    expect(redactTerminalOutput("OPENAI_API_KEY=sk-" + "x".repeat(30))).toContain("***");
    expect(redactTerminalOutput("xoxb-123456-abcdef")).toBe("***");
  });

  it("removes a private key block with its newlines", () => {
    const pem =
      "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nmore\n-----END OPENSSH PRIVATE KEY-----";
    expect(redactTerminalOutput(`before\n${pem}\nafter`)).toBe("before\n***\nafter");
  });

  it("keeps the scheme of a URL and blanks the credentials inside it", () => {
    expect(redactTerminalOutput("clone https://alice:s3cret@github.com/x/y.git")).toBe(
      "clone https://***@github.com/x/y.git",
    );
  });

  it("leaves ordinary output alone", () => {
    const plain = "total 48\ndrwxr-xr-x 5 root root 4096 Oct 1 12:00 .\n-rw-r--r-- 1 root root 220 Jan 6 token\n";
    expect(redactTerminalOutput(plain)).toBe(plain);
  });

  it("is idempotent, so a host scrubbing again cannot damage the result", () => {
    const once = redactTerminalOutput("password=hunter2 Bearer abc.def");
    expect(redactTerminalOutput(once)).toBe(once);
  });

  it("does not guess at a short option, because it cannot be told from a plain one", () => {
    // `mysql -ppass` and `find -print` are the same shape from here; mangling a command is worse
    // than missing its argument, and the preview is the boundary that catches what this does not.
    expect(redactTerminalOutput("mysql -phunter2")).toBe("mysql -phunter2");
  });
});

describe("stripAnsiEscapes", () => {
  it("removes an operating-system command report and the colour codes around it", () => {
    const noisy = "\u001b]633;D;2\u0007\u001b[0m$ \u001b[1;32m✓\u001b[0m ok\r\nnext";
    expect(stripAnsiEscapes(noisy)).toBe("$ ✓ ok\nnext");
  });

  it("removes both ways an operating-system command report can end", () => {
    expect(stripAnsiEscapes("\u001b]633;E;cmd\u0007out")).toBe("out");
    expect(stripAnsiEscapes("\u001b]633;E;cmd\u001b\\out")).toBe("out");
  });

  it("folds a progress line that redrew itself into one line", () => {
    expect(stripAnsiEscapes("10%\r20%\r100%\ndone")).toBe("10%20%100%\ndone");
  });
});

describe("prepareAiOutputSnapshot", () => {
  it("truncates before it redacts, so a value at the cut never survives", () => {
    const long = Array.from({ length: 300 }, (_, index) => (index === 299 ? "password=zoh" : `row-${index}`)).join(
      "\n",
    );
    const out = prepareAiOutputSnapshot(long);

    expect(out.split("\n")).toHaveLength(AI_OUTPUT_MAX_LINES);
    expect(out).toContain("password=***");
    expect(out).not.toContain("zoh");
  });
});

describe("prepareTerminalContext", () => {
  it("strips, truncates and redacts before the request budget is applied", () => {
    const noisy = `\u001b[31mexport TOKEN=abc\u001b[0m\nok`;
    expect(prepareTerminalContext(noisy, 80)).toBe("export TOKEN=***\nok");
  });

  it("attaches nothing when the setting asks for no lines", () => {
    expect(prepareTerminalContext("something", 0)).toBe("");
  });

  it("never sends more than the request budget", () => {
    const text = "y".repeat(60_000);
    expect(prepareTerminalContext(text, AI_OUTPUT_MAX_LINES, 1_000).length).toBeLessThanOrEqual(1_000);
  });

  it("holds the snapshot to the number of lines somebody asked for", () => {
    const text = Array.from({ length: 50 }, (_, index) => `row-${index}`).join("\n");
    expect(prepareTerminalContext(text, 10).split("\n")).toHaveLength(10);
  });
});

describe("countLines", () => {
  it("counts nothing for an empty snapshot", () => {
    expect(countLines("")).toBe(0);
    expect(countLines("one")).toBe(1);
    expect(countLines("one\ntwo")).toBe(2);
  });
});
