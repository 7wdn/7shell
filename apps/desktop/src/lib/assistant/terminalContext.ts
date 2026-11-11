/**
 * What the terminal's last lines look like just before they travel with a question.
 *
 * Portions of this file are derived from MIT-licensed code; see THIRD_PARTY_NOTICES.md.
 * Copyright (c) 2026 jinpy.he — released under the MIT License, reproduced in that file.
 *
 * Remote output leaving the machine is the one sensitive act this feature performs, so it happens
 * in one place and in one order, and every path that attaches a terminal goes through it. The four
 * steps are pure functions of a string, which is what makes the promise testable without a terminal:
 *
 *   strip control sequences → keep the tail → redact what looks like a credential → cap the budget
 *
 * Truncating before redacting is deliberate. The patterns run over less text, and an unterminated
 * key at the cut is not something the redactor has to reason about: what survives the cut is what
 * gets looked at.
 */

/**
 * The tail kept by the redaction pipeline itself: 200 lines and 16 KiB.
 *
 * A command's error sits at the end of its output, so the end is the part worth keeping. The line
 * bound is applied first because a line is the unit a person reads a log in; the character bound is
 * the backstop for a single enormous line.
 */
export const AI_OUTPUT_MAX_LINES = 200;
export const AI_OUTPUT_MAX_CHARS = 16 * 1024;

/** The last `maxLines` lines, then the last `maxChars` characters of those. */
export function truncateTail(text: string, maxLines = AI_OUTPUT_MAX_LINES, maxChars = AI_OUTPUT_MAX_CHARS): string {
  if (!text) {
    return "";
  }
  const lines = text.split("\n");
  const byLines = (lines.length > maxLines ? lines.slice(-maxLines) : lines).join("\n");
  return byLines.length > maxChars ? byLines.slice(-maxChars) : byLines;
}

/**
 * The patterns that turn a value into `***`, and why each one is here.
 *
 * The set is deliberately conservative: a false positive corrupts the text the model is asked to
 * reason about, which is worse than a shape nobody predicted slipping through — and the person sees
 * the exact text before it leaves, which is the real boundary.
 *
 * * **assignments keep their name.** `DB_PASSWORD=hunter2` becomes `DB_PASSWORD=***` rather than
 *   disappearing, because the field's *name* is what the answer has to talk about. Quotes and the
 *   separator are kept for the same reason.
 * * **standalone tokens are replaced whole**: `Bearer` credentials, AWS access key ids, the prefixes
 *   the large hosts use for personal tokens, and a private key block in either of its spellings.
 * * **a credential inside a URL loses only itself**, keeping the scheme so `https://user:pass@host`
 *   still reads as a URL.
 *
 * `***` matches none of the patterns, which is what makes the pipeline idempotent — redacting twice
 * is redacting once, so a host that scrubs again cannot damage what this pass produced.
 *
 * Short options such as `mysql -ppass` are deliberately absent: without knowing the program there is
 * no way to tell one from `find -print`, and mangling a command is worse than missing its argument.
 */
const REDACTION_RULES: ReadonlyArray<readonly [RegExp, string]> = [
  [
    /\b([a-z0-9_-]*(?:password|passwd|secret|token|api[_-]?key|apikey|access[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|private[_-]?key))(["']?)\s*([=:])(\s*)("?)\S[^\s"']*\5/gi,
    "$1$2$3$4***",
  ],
  [/\bBearer\s+\S+/g, "***"],
  [/\bAKIA[0-9A-Z]{16}\b/g, "***"],
  [/\b(?:ghp_|gho_|ghu_|ghs_)[A-Za-z0-9]{30,}\b/g, "***"],
  [/\bgithub_pat_[A-Za-z0-9_]{20,}\b/g, "***"],
  [/\bsk-[A-Za-z0-9]{20,}\b/g, "***"],
  [/\bxox[baprs]-[A-Za-z0-9-]{10,}\b/g, "***"],
  [/-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g, "***"],
  [/\b([a-z][a-z0-9+.-]*):\/\/[^\s:@/]+:[^\s@/]+@/gi, "$1://***@"],
];

/** Replaces every value the rules recognise, in the order the table lists them. */
export function redactTerminalOutput(text: string): string {
  if (!text) {
    return "";
  }
  let out = text;
  for (const [pattern, replacement] of REDACTION_RULES) {
    out = out.replace(pattern, replacement);
  }
  return out;
}

/** The one entry point for a buffer read: truncate, then redact. */
export function prepareAiOutputSnapshot(text: string): string {
  return redactTerminalOutput(truncateTail(text));
}

/**
 * Removes the escape sequences a terminal buffer still carries.
 *
 * The buffer this reads from is plain text with the sequences the shell emitted still in it, and to
 * a model they are noise: an operating-system command report or a cursor move arrives as opaque
 * bytes in the middle of a word. Carriage returns are folded away too, because a progress line that
 * redrew itself would otherwise arrive as one line of thirty copies of itself.
 */
export function stripAnsiEscapes(text: string): string {
  if (!text) {
    return "";
  }
  return (
    text
      // The escape character is the thing being removed, so matching a control character is the rule's
      // subject rather than a slip: without these two patterns the noise stays in the snapshot.
      // eslint-disable-next-line no-control-regex
      .replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)/g, "")
      // eslint-disable-next-line no-control-regex
      .replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "")
      .replace(/\r/g, "")
  );
}

/**
 * The whole pipeline for one snapshot, with the question's own budget on the end.
 *
 * `lines` is the number the person asked for, applied again here rather than trusted to the caller:
 * a buffer read and this function both bound the snapshot, and a caller that passed a longer string
 * must not be able to send more than the setting says. `maxChars` is the request budget from the
 * contract rather than the pipeline's — the two bounds answer different questions, "how much of a
 * log is worth reading" and "how much of a request may carry it".
 */
export function prepareTerminalContext(text: string, lines: number, maxChars?: number): string {
  if (!text || lines <= 0) {
    return "";
  }
  const prepared = redactTerminalOutput(truncateTail(stripAnsiEscapes(text), Math.min(lines, AI_OUTPUT_MAX_LINES)));
  if (maxChars === undefined || prepared.length <= maxChars) {
    return prepared;
  }
  return prepared.slice(-maxChars);
}

/** How many lines a snapshot holds, which is what the preview says was attached. */
export function countLines(text: string): number {
  return text === "" ? 0 : text.split("\n").length;
}
