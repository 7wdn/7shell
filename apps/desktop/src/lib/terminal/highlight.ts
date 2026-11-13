/**
 * Finding the words a rule names, in one line the terminal has already drawn.
 *
 * Kept away from the terminal itself so the matching can be tested without a laid-out document.
 * The rules are the person's own: a plain word is escaped and an explicit regular expression is
 * taken as one, and a pattern that does not compile is skipped rather than breaking the rest of the
 * list. One line never carries more than `perLine` marks — a rule like `.` would otherwise put a
 * decoration on every character.
 */
import type { TerminalHighlightRule } from "@/types/app";

export interface CompiledHighlightRule {
  readonly rule: TerminalHighlightRule;
  readonly regex: RegExp;
}

export interface HighlightMatch {
  readonly start: number;
  readonly end: number;
  readonly color: string;
  readonly ruleId: string;
}

/** What a line may carry, and what the whole scrollback may carry. */
export const HIGHLIGHT_MATCHES_PER_LINE = 20;
export const HIGHLIGHT_DECORATION_LIMIT = 400;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function compileHighlightRules(rules: readonly TerminalHighlightRule[]): readonly CompiledHighlightRule[] {
  const compiled: CompiledHighlightRule[] = [];
  for (const rule of rules) {
    const pattern = rule.pattern.trim();
    if (!pattern) {
      continue;
    }
    const source = rule.isRegex ? pattern : escapeRegExp(pattern);
    try {
      compiled.push({
        rule,
        regex: new RegExp(source, `g${rule.caseSensitive ? "" : "i"}`),
      });
    } catch {
      // A pattern that is not a regular expression is a rule this line cannot apply. The editor
      // says so where it is written; here the rest of the list still works.
    }
  }
  // The longest pattern is tried first, so "Permission denied" (red) wins over "denied" (amber)
  // inside the same phrase instead of being painted over by it.
  return compiled.sort((a, b) => b.rule.pattern.trim().length - a.rule.pattern.trim().length);
}

/**
 * Every match of every compiled rule in one line.
 *
 * Rules are tried longest-first and an earlier match keeps its characters: a later rule whose hit
 * overlaps one already taken is skipped. The line is bounded so a pattern like `.` cannot turn the
 * terminal into a drawing exercise. Zero-width hits advance the expression by one and are not
 * drawn — a mark on nothing is a mark nobody can see anyway.
 */
export function highlightMatches(
  line: string,
  compiled: readonly CompiledHighlightRule[],
  perLine = HIGHLIGHT_MATCHES_PER_LINE,
): readonly HighlightMatch[] {
  if (!line || compiled.length === 0 || perLine <= 0) {
    return [];
  }
  const taken: [start: number, end: number][] = [];
  const matches: HighlightMatch[] = [];
  for (const { rule, regex } of compiled) {
    regex.lastIndex = 0;
    let hit: RegExpExecArray | null;
    while ((hit = regex.exec(line)) !== null) {
      if (hit[0].length === 0) {
        regex.lastIndex += 1;
        continue;
      }
      const start = hit.index;
      const end = start + hit[0].length;
      const overlaps = taken.some(([takenStart, takenEnd]) => start < takenEnd && end > takenStart);
      if (!overlaps) {
        taken.push([start, end]);
        matches.push({ start, end, color: rule.color, ruleId: rule.id });
        if (matches.length >= perLine) {
          return matches.sort((a, b) => a.start - b.start);
        }
      }
    }
  }
  return matches.sort((a, b) => a.start - b.start);
}
