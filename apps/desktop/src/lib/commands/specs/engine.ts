/**
 * Reading a command line against a specification.
 *
 * The specification says what a command accepts: subcommands, options, positional arguments, and the
 * words or generated values each of those can be. This module walks the words before the caret
 * through that shape and answers what could come next at the caret. It is a pure reading of the line
 * and the data — the only thing it can ask the outside world for is a generated value list, and even
 * that arrives through a cache the caller owns.
 *
 * The interface's own sources (saved lines, history, the host's directories) are not lost when a
 * specification exists: this engine answers the grammar, and the caller merges that answer with the
 * others.
 */
import { commandBefore, scan, type ScannedWord } from "@/lib/commands/tokenize";

import type { SpecArg, SpecCatalog, SpecCommand, SpecGenerator, SpecOption, SpecSuggestion } from "./types";

export interface SpecCandidate {
  readonly label: string;
  readonly insert: string;
  readonly detail: string | null;
  readonly kind: "command" | "subcommand" | "option" | "argument";
  /** Where the accepted text replaces up to the caret. */
  readonly from: number;
  readonly score: number;
  /** Whether taking this candidate should be followed by a space. */
  readonly trailingSpace: boolean;
  readonly labelRanges: readonly { readonly start: number; readonly end: number }[];
}

export interface SpecValueItem {
  readonly label: string;
  readonly detail?: string;
}

/** One generator the caller should run, and the cache key its answer belongs under. */
export interface SpecGeneratorSlot {
  readonly key: string;
  readonly handle: string;
  readonly command: string;
  readonly args: readonly string[];
  readonly cwd: string | null;
  readonly splitOn: string | null;
  readonly tokenStart: number;
  readonly caret: number;
  readonly prefix: string;
}

/**
 * Generated values, as the caller has them.
 *
 * `cached` answering `null` means "not read yet": the caller should run the slot and ask again on
 * the next build. `warm` is the request to do that — it is a side effect on purpose, the same shape
 * the file panel's directory source already uses.
 */
export interface SpecValueSource {
  cached(key: string): readonly SpecValueItem[] | null;
  warm(slot: SpecGeneratorSlot): void;
}

export interface SpecCompletionRequest {
  readonly line: string;
  readonly caret: number;
  readonly catalog: SpecCatalog;
  /** The terminal whose connection would run a generator. */
  readonly handle: string;
  /** The shell's directory, when it is known; a generator runs there. */
  readonly cwd: string | null;
  readonly values?: SpecValueSource;
  /** Command names from the interface's own catalog, for arguments that take a command word. */
  readonly commandNames?: readonly string[];
  readonly limit?: number;
}

export interface SpecCompletionResult {
  readonly candidates: readonly SpecCandidate[];
  /** An argument at the caret wants a file or folder list. */
  readonly wantsPath: boolean;
}

/** The limit on one specification's own answer before the caller merges in the other sources. */
const DEFAULT_LIMIT = 80;

/** Scores before matching: the order the kinds are worth reading in. */
const SCORE = { subcommand: 440, option: 420, argument: 380, command: 90 } as const;

interface Pending {
  readonly label: string;
  readonly insert: string;
  readonly detail: string | null;
  readonly kind: SpecCandidate["kind"];
  readonly score: number;
  readonly trailingSpace: boolean;
}

/**
 * Walks the line against the specification. `null` means this command has no specification, which is
 * the caller's cue to fall back to the hand-written table and to its own sources.
 */
export function buildSpecCompletions(request: SpecCompletionRequest): SpecCompletionResult | null {
  const words = scan(request.line, request.caret);
  const last = words.at(-1);
  if (last?.comment === true) {
    return null;
  }
  const current =
    last !== undefined && !last.separator && last.end >= request.caret && last.end > last.start ? last : null;

  if (current !== null && (current.text.startsWith("'") || current.text.startsWith('"'))) {
    return null;
  }

  const segment = segmentWords(words, current === null ? words.length : words.length - 1);
  const before = segment.filter((word) => !word.separator && !word.comment);
  const { command, at } = commandBefore(before);
  if (command === null || at < 0) {
    return null;
  }

  const root = request.catalog.byName.get(command.toLowerCase());
  if (root === undefined) {
    return null;
  }

  const prefix = current === null ? "" : decodeWord(current.text);
  const from = current === null ? request.caret : current.start;
  const tokens = before.slice(at + 1).map((word) => decodeWord(word.text));

  const state = walk(root, tokens, request.catalog);
  const out: Pending[] = [];
  let wantsPath = false;

  if (state.pending !== null) {
    const arg = state.pending.arg;
    addSuggestions(out, arg.suggestions, prefix, SCORE.argument);
    wantsPath ||= argWantsPath(arg);
    addGenerators(out, arg, request, from, prefix);
  } else {
    if (!state.enteredArgs && !state.endOfOptions) {
      addSubcommands(out, state.spec, prefix);
    }
    if (!state.endOfOptions) {
      addOptions(out, state.spec, prefix);
    }

    const arg = currentArg(state);
    if (arg !== null) {
      addSuggestions(out, arg.suggestions, prefix, SCORE.argument);
      wantsPath ||= argWantsPath(arg);
      addGenerators(out, arg, request, from, prefix);
      if (arg.isCommand === true) {
        addCommandNames(out, request.commandNames ?? [], prefix);
      }
    }

    addSuggestions(out, state.spec.additionalSuggestions, prefix, SCORE.argument);
  }

  const candidates = dedupe(out)
    .sort(compare)
    .slice(0, request.limit ?? DEFAULT_LIMIT)
    .map((pending) => toCandidate(pending, prefix, from));

  return { candidates, wantsPath };
}

/** Command names for a partly typed command word; the caller decides when to ask for this. */
export function specCommandCandidates(
  catalog: SpecCatalog,
  prefix: string,
  from: number,
  limit = DEFAULT_LIMIT,
): SpecCandidate[] {
  const out: Pending[] = [];
  const needle = prefix.toLowerCase();
  for (const name of catalog.commandNames) {
    if (!name.toLowerCase().startsWith(needle)) {
      continue;
    }
    const description = catalog.byName.get(name.toLowerCase())?.description ?? null;
    out.push({
      label: name,
      insert: name,
      detail: description,
      kind: "command",
      score: SCORE.command + (name.toLowerCase() === needle ? 200 : 0),
      trailingSpace: true,
    });
  }
  return dedupe(out)
    .sort(compare)
    .slice(0, limit)
    .map((pending) => toCandidate(pending, prefix, from));
}

// ---------------------------------------------------------------------------
// Walking the specification
// ---------------------------------------------------------------------------

interface WalkState {
  spec: SpecCommand;
  argIndex: number;
  pending: { option: SpecOption; arg: SpecArg; index: number } | null;
  endOfOptions: boolean;
  enteredArgs: boolean;
}

function walk(root: SpecCommand, tokens: readonly string[], catalog: SpecCatalog): WalkState {
  const state: WalkState = {
    spec: resolveSpec(root, catalog),
    argIndex: 0,
    pending: null,
    endOfOptions: false,
    enteredArgs: false,
  };

  for (const token of tokens) {
    if (state.pending !== null) {
      // An optional value that would be a flag is not a value: the person moved on to the next
      // option, and the parser must not swallow it as this option's argument.
      if (state.pending.arg.isOptional === true && isOptionToken(token)) {
        state.pending = null;
      } else {
        consumePending(state);
        continue;
      }
    }
    if (!state.endOfOptions && token === "--") {
      state.endOfOptions = true;
      continue;
    }
    if (!state.endOfOptions && isOptionToken(token)) {
      const match = matchOption(state.spec, token);
      if (match !== null) {
        const args = match.option.args ?? [];
        if (args.length > 0) {
          state.pending = { option: match.option, arg: args[0]!, index: 0 };
          if (match.inlineValue !== null) {
            consumePending(state);
          }
        }
      }
      continue;
    }

    if (!state.enteredArgs) {
      const subcommand = findSubcommand(state.spec, token);
      if (subcommand !== undefined) {
        state.spec = resolveSpec(subcommand, catalog);
        state.argIndex = 0;
        state.pending = null;
        continue;
      }
    }

    state.enteredArgs = true;
    state.argIndex += 1;
  }

  return state;
}

function currentArg(state: WalkState): SpecArg | null {
  const args = state.spec.args;
  if (args === undefined || args.length === 0) {
    return null;
  }
  const at = state.argIndex;
  if (at >= args.length) {
    const last = args[args.length - 1];
    return last?.isVariadic === true ? last : null;
  }
  return args[at] ?? null;
}

function consumePending(state: WalkState): void {
  const pending = state.pending;
  if (pending === null) {
    return;
  }
  if (pending.arg.isVariadic === true) {
    return;
  }
  const next = pending.index + 1;
  if (next < (pending.option.args?.length ?? 0)) {
    state.pending = { option: pending.option, arg: pending.option.args![next]!, index: next };
  } else {
    state.pending = null;
  }
}

function resolveSpec(spec: SpecCommand, catalog: SpecCatalog): SpecCommand {
  if (typeof spec.loadSpec !== "string") {
    return spec;
  }
  return catalog.registry[spec.loadSpec] ?? spec;
}

function findSubcommand(spec: SpecCommand, token: string): SpecCommand | undefined {
  for (const subcommand of spec.subcommands ?? []) {
    if (subcommand.names.includes(token)) {
      return subcommand;
    }
  }
  return undefined;
}

interface OptionMatch {
  readonly option: SpecOption;
  /** The value written after `=` or inside a short option, when there is one. */
  readonly inlineValue: string | null;
}

function matchOption(spec: SpecCommand, token: string): OptionMatch | null {
  for (const option of spec.options ?? []) {
    if (option.names.includes(token)) {
      return { option, inlineValue: null };
    }
  }

  if (token.startsWith("--")) {
    const equals = token.indexOf("=");
    if (equals > 2) {
      const name = token.slice(0, equals);
      const value = token.slice(equals + 1);
      for (const option of spec.options ?? []) {
        if (option.names.includes(name)) {
          return { option, inlineValue: value };
        }
      }
    }
    return null;
  }

  // A short option cluster: `-la`, `-n5`, `-C/path`. The first known option that takes an argument
  // owns the rest of the token; the others are flags and are consumed without state.
  if (token.startsWith("-") && token.length > 2) {
    for (const option of spec.options ?? []) {
      const short = option.names.find((name) => name.startsWith("-") && !name.startsWith("--"));
      if (short === undefined || !token.startsWith(short)) {
        continue;
      }
      if (option.args !== undefined && option.args.length > 0) {
        return { option, inlineValue: token.slice(short.length) };
      }
    }
  }

  return null;
}

function isOptionToken(token: string): boolean {
  return token.startsWith("-") && token !== "-" && !/^-\d/.test(token);
}

// ---------------------------------------------------------------------------
// Sources of candidates at the caret
// ---------------------------------------------------------------------------

function addSubcommands(out: Pending[], spec: SpecCommand, prefix: string): void {
  for (const subcommand of spec.subcommands ?? []) {
    const names = prefix === "" ? subcommand.names.slice(0, 1) : subcommand.names;
    for (const name of names) {
      if (!startsWith(name, prefix)) {
        continue;
      }
      out.push({
        label: name,
        insert: name,
        detail: subcommand.description ?? null,
        kind: "subcommand",
        score: SCORE.subcommand + (name.toLowerCase() === prefix.toLowerCase() ? 200 : 0),
        trailingSpace: true,
      });
    }
  }
}

function addOptions(out: Pending[], spec: SpecCommand, prefix: string): void {
  for (const option of spec.options ?? []) {
    const name = optionSpelling(option, prefix);
    if (name === null) {
      continue;
    }
    out.push({
      label: name,
      insert: name,
      detail: option.description ?? null,
      kind: "option",
      score: SCORE.option + (name.toLowerCase() === prefix.toLowerCase() ? 200 : 0),
      trailingSpace: true,
    });
  }
}

function optionSpelling(option: SpecOption, prefix: string): string | null {
  if (prefix === "") {
    return option.names.find((name) => name.startsWith("--")) ?? option.names[0] ?? null;
  }
  if (prefix.startsWith("--")) {
    return option.names.find((name) => name.startsWith("--") && startsWith(name, prefix)) ?? null;
  }
  return (
    option.names.find((name) => !name.startsWith("--") && startsWith(name, prefix)) ??
    option.names.find((name) => startsWith(name, prefix)) ??
    null
  );
}

function addSuggestions(
  out: Pending[],
  suggestions: readonly SpecSuggestion[] | undefined,
  prefix: string,
  baseScore: number,
): void {
  for (const suggestion of suggestions ?? []) {
    if (suggestion.hidden === true) {
      continue;
    }
    const names = prefix === "" ? suggestion.name.slice(0, 1) : suggestion.name;
    for (const name of names) {
      if (!startsWith(name, prefix)) {
        continue;
      }
      out.push({
        label: name,
        insert: name,
        detail: suggestion.description ?? null,
        kind: "argument",
        score: baseScore + (name.toLowerCase() === prefix.toLowerCase() ? 200 : 0),
        trailingSpace: !name.endsWith("/") && !name.endsWith("="),
      });
    }
  }
}

function addCommandNames(out: Pending[], names: readonly string[], prefix: string): void {
  for (const name of names) {
    if (!startsWith(name, prefix)) {
      continue;
    }
    out.push({
      label: name,
      insert: name,
      detail: null,
      kind: "command",
      score: SCORE.command,
      trailingSpace: true,
    });
  }
}

/** A generated value list, when the caller has it already, and the request to read it when not. */
function addGenerators(
  out: Pending[],
  arg: SpecArg,
  request: SpecCompletionRequest,
  from: number,
  prefix: string,
): void {
  for (const generator of arg.generators ?? []) {
    const slot = slotFor(generator, request, from, prefix);
    if (slot === null) {
      continue;
    }
    if (request.values === undefined) {
      continue;
    }
    const values = request.values.cached(slot.key);
    if (values === null) {
      request.values.warm(slot);
      continue;
    }
    for (const value of values) {
      if (!startsWith(value.label, prefix)) {
        continue;
      }
      out.push({
        label: value.label,
        insert: value.label,
        detail: value.detail ?? null,
        kind: "argument",
        // Above generic path entries: a branch or container the host just named is a better answer
        // than the directory listing that happens to be available at the same position.
        score: SCORE.argument + 20,
        trailingSpace: true,
      });
    }
  }
}

/** The cache key deliberately leaves the prefix out: one read answers every prefix of the token. */
export function generatorKey(
  handle: string,
  cwd: string | null,
  script: readonly string[],
  tokenStart: number,
): string {
  return JSON.stringify([handle, cwd, script, tokenStart]);
}

function slotFor(
  generator: SpecGenerator,
  request: SpecCompletionRequest,
  from: number,
  prefix: string,
): SpecGeneratorSlot | null {
  const script = generator.script;
  if (script === undefined || script.length === 0 || script[0] === undefined || script[0] === "") {
    return null;
  }
  return {
    key: generatorKey(request.handle, request.cwd, script, from),
    handle: request.handle,
    command: script[0],
    args: script.slice(1),
    cwd: request.cwd,
    splitOn: typeof generator.splitOn === "string" ? generator.splitOn : null,
    tokenStart: from,
    caret: request.caret,
    prefix,
  };
}

function argWantsPath(arg: SpecArg): boolean {
  if (arg.template !== undefined) {
    const templates = typeof arg.template === "string" ? [arg.template] : arg.template;
    if (templates.some((template) => template === "filepaths" || template === "folders")) {
      return true;
    }
  }
  for (const generator of arg.generators ?? []) {
    if (generator.template === undefined) {
      continue;
    }
    const templates = typeof generator.template === "string" ? [generator.template] : generator.template;
    if (templates.some((template) => template === "filepaths" || template === "folders")) {
      return true;
    }
  }
  return (arg.suggestions ?? []).some((suggestion) => suggestion.type === "file" || suggestion.type === "folder");
}

// ---------------------------------------------------------------------------
// Matching, ordering and the final shape
// ---------------------------------------------------------------------------

function startsWith(text: string, prefix: string): boolean {
  return prefix === "" || text.toLowerCase().startsWith(prefix.toLowerCase());
}

function dedupe(pending: readonly Pending[]): Pending[] {
  const byInsert = new Map<string, Pending>();
  for (const candidate of pending) {
    const kept = byInsert.get(candidate.insert);
    if (kept === undefined || candidate.score > kept.score) {
      byInsert.set(candidate.insert, candidate);
    }
  }
  return [...byInsert.values()];
}

function compare(left: Pending, right: Pending): number {
  if (left.score !== right.score) {
    return right.score - left.score;
  }
  return left.label.localeCompare(right.label);
}

function toCandidate(pending: Pending, prefix: string, from: number): SpecCandidate {
  const labelRanges = pending.label.toLowerCase().startsWith(prefix.toLowerCase())
    ? [{ start: 0, end: Math.min(prefix.length, pending.label.length) }]
    : [];
  return {
    label: pending.label,
    insert: pending.insert,
    detail: pending.detail,
    kind: pending.kind,
    from,
    score: pending.score,
    trailingSpace: pending.trailingSpace,
    labelRanges,
  };
}

/** The words of the segment the caret is in, after its last separator. */
function segmentWords(words: readonly ScannedWord[], upTo: number): ScannedWord[] {
  let start = 0;
  for (let index = upTo - 1; index >= 0; index -= 1) {
    if (words[index]?.separator === true) {
      start = index + 1;
      break;
    }
  }
  return words.slice(start, upTo);
}

/**
 * A word's value with the shell's quoting taken off.
 *
 * Enough for matching names and flags: single quotes are literal, double quotes and bare text lose
 * their backslash escapes. What the shell would expand is not this side's business — a candidate
 * built from an expansion would be a guess.
 */
export function decodeWord(raw: string): string {
  let out = "";
  let quote: "'" | '"' | null = null;
  for (let index = 0; index < raw.length; index += 1) {
    const character = raw[index] ?? "";
    if (quote === "'") {
      if (character === "'") {
        quote = null;
      } else {
        out += character;
      }
      continue;
    }
    if (character === "\\") {
      const next = raw[index + 1];
      if (next !== undefined) {
        out += next;
        index += 1;
      }
      continue;
    }
    if (quote === '"') {
      if (character === '"') {
        quote = null;
      } else {
        out += character;
      }
      continue;
    }
    if (character === "'" || character === '"') {
      quote = character;
      continue;
    }
    out += character;
  }
  return out;
}
