/**
 * The three rules the external-server forms are built on, held still.
 *
 * Each one is something the form would get wrong silently: an argument split on spaces, an id the
 * shell would refuse only after the round trip, a secret row that becomes a nameless value. None of
 * them needs a page to test, which is why they live as functions.
 */
import { describe, expect, it } from "vitest";

import { keyValueRecord, parseServerArgs } from "@/lib/mcp/servers";
import { MCP_SERVER_ID_MAX_CHARS, isValidMcpServerId } from "@/types/mcp";

describe("the arguments of a stdio server", () => {
  it("keeps one argument per line, spaces and all", () => {
    expect(parseServerArgs("-y\n@modelcontextprotocol/server-filesystem\n/tmp/a b")).toEqual([
      "-y",
      "@modelcontextprotocol/server-filesystem",
      "/tmp/a b",
    ]);
  });

  it("reads an empty editor as no arguments at all", () => {
    expect(parseServerArgs("")).toEqual([]);
    expect(parseServerArgs("\n  \n\t\n")).toEqual([]);
  });

  it("ignores windows line endings and the whitespace around each line", () => {
    expect(parseServerArgs("  --port\r\n  8080  ")).toEqual(["--port", "8080"]);
  });
});

describe("the id a server is named by", () => {
  it("accepts what the shell accepts, and only that", () => {
    expect(isValidMcpServerId("files")).toBe(true);
    expect(isValidMcpServerId("files-2_x")).toBe(true);
    expect(isValidMcpServerId("_")).toBe(true);
    // One to 32 characters.
    expect(isValidMcpServerId("")).toBe(false);
    expect(isValidMcpServerId("a".repeat(MCP_SERVER_ID_MAX_CHARS))).toBe(true);
    expect(isValidMcpServerId("a".repeat(MCP_SERVER_ID_MAX_CHARS + 1))).toBe(false);
    // Tool names parse on the double underscore, so an id cannot carry one.
    expect(isValidMcpServerId("a__b")).toBe(false);
    expect(isValidMcpServerId("files.server")).toBe(false);
    expect(isValidMcpServerId("files server")).toBe(false);
    expect(isValidMcpServerId("文件")).toBe(false);
  });
});

describe("the rows a server's secrets are typed into", () => {
  it("skips the empty row an editor opens with", () => {
    expect(keyValueRecord([{ key: "", value: "" }])).toEqual({});
  });

  it("keeps a value that is deliberately empty", () => {
    expect(keyValueRecord([{ key: "NO_PROXY", value: "" }])).toEqual({ NO_PROXY: "" });
  });

  it("trims the name and leaves the value exactly as typed", () => {
    expect(keyValueRecord([{ key: "  TOKEN ", value: " a b " }])).toEqual({ TOKEN: " a b " });
  });

  it("refuses a value with no name", () => {
    expect(keyValueRecord([{ key: "", value: "orphan" }])).toBeNull();
  });

  it("refuses two rows with one name, rather than keeping the last", () => {
    expect(
      keyValueRecord([
        { key: "TOKEN", value: "one" },
        { key: "TOKEN", value: "two" },
      ]),
    ).toBeNull();
  });

  it("keeps two names that only differ by case, which are two different names", () => {
    expect(
      keyValueRecord([
        { key: "token", value: "one" },
        { key: "TOKEN", value: "two" },
      ]),
    ).toEqual({ token: "one", TOKEN: "two" });
  });
});
