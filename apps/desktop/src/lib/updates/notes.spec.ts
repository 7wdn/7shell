/**
 * Reading a release body.
 *
 * What matters here is not that markdown is honoured but that it is *bounded*: the subset a changelog
 * is written in comes out as blocks and runs, everything else comes out as the text it is, and a link
 * that does not point at the web never becomes a link. The notes arrive from a release server, so the
 * second half is the part that keeps them from being markup in the window.
 */
import { describe, expect, it } from "vitest";

import { parseNotes } from "@/lib/updates/notes";

describe("a release body, read", () => {
  it("separates headings, bullets and paragraphs", () => {
    const blocks = parseNotes(
      ["## What changed", "", "- one thing", "- another thing", "", "A closing sentence."].join("\n"),
    );

    expect(blocks.map((block) => block.kind)).toEqual(["heading", "bullets", "paragraph"]);
    const bullets = blocks[1];
    expect(bullets?.kind === "bullets" && bullets.items.length).toBe(2);
    expect(bullets?.kind === "bullets" && bullets.ordered).toBe(false);
  });

  it("keeps consecutive bullets in the list they belong to, wrapped lines included", () => {
    const blocks = parseNotes("- first\n  continued here\n- second");

    expect(blocks).toHaveLength(1);
    const bullets = blocks[0];
    expect(bullets?.kind === "bullets" && bullets.items.length).toBe(2);
    const first = bullets?.kind === "bullets" ? bullets.items[0] : undefined;
    expect(first?.map((span) => span.text).join("")).toBe("first continued here");
  });

  it("reads a numbered list as numbered rather than as bullets", () => {
    const blocks = parseNotes("1. first\n2. second");

    expect(blocks[0]?.kind === "bullets" && blocks[0].ordered).toBe(true);
  });

  it("reads bold and inline code as runs of their own", () => {
    const blocks = parseNotes("A **bold** word and a `code` one.");

    const spans = blocks[0]?.kind === "paragraph" ? blocks[0].spans : [];
    expect(spans.map((span) => span.kind)).toEqual(["text", "strong", "text", "code", "text"]);
  });

  it("turns a link to the web into a link, and nothing else into one", () => {
    const web = parseNotes("See [the release](https://example.com/notes).");
    const spans = web[0]?.kind === "paragraph" ? web[0].spans : [];
    expect(spans.some((span) => span.kind === "link" && span.href === "https://example.com/notes")).toBe(true);

    // The case this exists for: a body that tries to make the window do something else. A reader
    // needs to see the text that was written, not a link that goes somewhere it should not.
    for (const attempt of ["[click](javascript:alert(1))", "[here](file:///etc/passwd)", "[x](data:text/html,y)"]) {
      const blocks = parseNotes(attempt);
      const runs = blocks[0]?.kind === "paragraph" ? blocks[0].spans : [];
      expect(
        runs.every((span) => span.kind !== "link"),
        `${attempt} became a link`,
      ).toBe(true);
    }
  });

  it("shows a shape it does not know as the text it is", () => {
    const blocks = parseNotes("| a | table |\n> quoted\n~~~\ncode fence\n~~~");

    // Four paragraphs of literal text rather than a table, a quote and a fence nobody implemented.
    expect(blocks.every((block) => block.kind === "paragraph")).toBe(true);
    expect(blocks[0]?.kind === "paragraph" && blocks[0].spans[0]?.text).toBe("| a | table |");
  });

  it("says nothing at all about an empty body", () => {
    expect(parseNotes("")).toEqual([]);
    expect(parseNotes("\n \n")).toEqual([]);
  });
});
