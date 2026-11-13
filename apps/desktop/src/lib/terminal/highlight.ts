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
    if (!rule.pattern.trim()) {
      continue;
    }
    const source = rule.isRegex ? rule.pattern : escapeRegExp(rule.pattern);
    try {
      compiled.push({
        rule,
        regex: new RegExp(source, `g${rule.caseSensitive ? "" : "i"}u`),
      });
    } catch {
      // A pattern that is not a regular expression is a rule this line cannot apply. The editor
      // says so where it is written; here the rest of the list still works.
    }
  }
  return compiled;
}

/** Every match of every compiled rule in one line, in the order the rules were written. */
export function highlightMatches(
  line: string,
  compiled: readonly CompiledHighlightRule[],
  perLine = HIGHLIGHT_MATCHES_PER_LINE,
): readonly HighlightMatch[] {
  const matches: HighlightMatch[] = [];
  for (const { rule, regex } of compiled) {
    for (const match of line.matchAll(regex)) {
      const start = match.index ?? 0;
      const end = start + match[0].length;
      if (end <= start) {
        continue;
      }
      matches.push({ start, end, color: rule.color, ruleId: rule.id });
      if (matches.length >= perLine) {
        return matches;
      }
    }
  }
  return matches;
}
