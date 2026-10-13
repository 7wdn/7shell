/**
 * A release's notes, as much of them as a dialog needs.
 *
 * A release body is written by hand in the changelog and published as it stands, so what arrives is a
 * few headings, some bullets, and the occasional bolded or linked word — which is what this reads.
 * It is deliberately not a markdown parser: the shapes it does not know are shown as the text they
 * are, because a release note that lost half of itself to a parser nobody tested is worse than one
 * that shows a stray asterisk.
 *
 * Returning structures rather than markup is the other half of the decision: the notes are drawn by
 * the component that renders them, so nothing here can end up as HTML in the window. Release notes
 * are text from a server, and text from a server does not get to be markup in this application.
 */

/** A run of text inside a line, and what it is drawn as. */
export type NoteSpan =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "strong"; readonly text: string }
  | { readonly kind: "code"; readonly text: string }
  | { readonly kind: "link"; readonly text: string; readonly href: string };

/** One block of a release note. */
export type NoteBlock =
  | { readonly kind: "heading"; readonly level: number; readonly spans: readonly NoteSpan[] }
  | { readonly kind: "paragraph"; readonly spans: readonly NoteSpan[] }
  | { readonly kind: "bullets"; readonly items: readonly (readonly NoteSpan[])[]; readonly ordered: boolean };

/** The address a link may point at: the web, and nothing else. */
const LINKABLE = /^https?:\/\/[^\s]+$/;

/**
 * Reads the notes into blocks.
 *
 * Blank lines separate blocks, which is the one rule that matters for a body written by a person. A
 * line that starts with `#` is a heading, `-` or `*` or `1.` starts a list — consecutive list lines
 * become one list — and everything else is a paragraph.
 */
export function parseNotes(markdown: string): NoteBlock[] {
  const blocks: NoteBlock[] = [];
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n");

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]?.trimEnd() ?? "";
    if (line.trim() === "") {
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1]?.length ?? 1, spans: parseSpans(heading[2] ?? "") });
      continue;
    }

    const bullet = /^\s*([-*]|\d+\.)\s+(.*)$/.exec(line);
    if (bullet) {
      const ordered = (bullet[1] ?? "").endsWith(".");
      const items: NoteSpan[][] = [parseSpans(bullet[2] ?? "")];
      // A wrapped line — indented and not a list marker of its own — belongs to the item above it.
      while (index + 1 < lines.length) {
        const next = lines[index + 1] ?? "";
        if (/^\s+\S/.test(next) && !/^\s*([-*]|\d+\.)\s+/.test(next)) {
          // A wrapped line follows the one above it, and the break it was wrapped at is a space in
          // the sentence — which is what the reader of the rendered line has to see.
          items[items.length - 1]?.push({ kind: "text", text: " " }, ...parseSpans(next.trim()));
          index += 1;
          continue;
        }
        const following = /^\s*([-*]|\d+\.)\s+(.*)$/.exec(next);
        if (following && following[1]?.endsWith(".") === ordered) {
          items.push(parseSpans(following[2] ?? ""));
          index += 1;
          continue;
        }
        break;
      }
      const last = blocks[blocks.length - 1];
      if (last?.kind === "bullets" && last.ordered === ordered) {
        blocks[blocks.length - 1] = { kind: "bullets", ordered, items: [...last.items, ...items] };
      } else {
        blocks.push({ kind: "bullets", ordered, items });
      }
      continue;
    }

    blocks.push({ kind: "paragraph", spans: parseSpans(line.trim()) });
  }

  return blocks;
}

/**
 * Reads one line into runs.
 *
 * Three shapes, in the order they are looked for: a link, bold, and inline code. Anything else is
 * text — including the parts of a shape that was opened and never closed, which is why the
 * patterns require both ends on the same line.
 */
function parseSpans(text: string): NoteSpan[] {
  const spans: NoteSpan[] = [];
  const pattern = /\[([^\]]+)\]\(([^)\s]+)\)|\*\*([^*]+)\*\*|`([^`]+)`/g;
  let cursor = 0;

  for (let match = pattern.exec(text); match !== null; match = pattern.exec(text)) {
    if (match.index > cursor) {
      spans.push({ kind: "text", text: text.slice(cursor, match.index) });
    }

    const [, label, href, strong, code] = match;
    if (label !== undefined && href !== undefined) {
      // A link is a link only when it goes somewhere the system's browser can be asked to open;
      // anything else is shown as the text it was written as, which is what a reader can act on.
      spans.push(LINKABLE.test(href) ? { kind: "link", text: label, href } : { kind: "text", text: match[0] });
    } else if (strong !== undefined) {
      spans.push({ kind: "strong", text: strong });
    } else if (code !== undefined) {
      spans.push({ kind: "code", text: code });
    }

    cursor = match.index + match[0].length;
  }

  if (cursor < text.length) {
    spans.push({ kind: "text", text: text.slice(cursor) });
  }

  return spans;
}
