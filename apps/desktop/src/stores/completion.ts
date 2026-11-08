/**
 * Generated completion values, read on the terminal's own connection.
 *
 * A specification can say that an argument's values come from a program: `git checkout` is followed
 * by a branch, `docker logs` by a container. The program runs on the machine behind the terminal,
 * through the typed completion command, and its answer is cached here for the few hundred
 * milliseconds in which somebody is still typing the same word. The cache is shared by the command
 * bar and the terminal's own line, so the two surfaces cannot ask the same question twice.
 *
 * A list that came back empty or failed is remembered for longer than a list with values: a command
 * nobody has installed should not be asked about on every keystroke.
 */
import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import type { SpecGeneratorSlot, SpecValueItem } from "@/lib/commands/specs/engine";

// A generated list is about a directory or a set of branches, not about one keystroke: a person who
// pauses for a second is still asking the same question, and an SSH round trip repeated on every
// pause would make the menu flicker and the host work for nothing. Empty answers are remembered
// longer still, because a command nobody has installed will not appear on the next keystroke.
const VALUE_TTL_MS = 10_000;
const EMPTY_TTL_MS = 30_000;
const MAX_VALUES = 500;
const TIMEOUT_MS = 1_200;
const MAX_OUTPUT_BYTES = 256 * 1024;

interface Entry {
  readonly expiresAt: number;
  readonly values: readonly SpecValueItem[];
}

export const useCompletionStore = defineStore("completion", () => {
  /** Bumped when an answer arrives, so a surface showing a line can ask again. */
  const version = ref(0);
  const cache = new Map<string, Entry>();
  const pending = new Set<string>();

  /** The answer when it is here and still fresh; `null` when it has to be read. */
  function cached(key: string): readonly SpecValueItem[] | null {
    const entry = cache.get(key);
    if (entry === undefined) {
      return null;
    }
    if (entry.expiresAt <= Date.now()) {
      cache.delete(key);
      return null;
    }
    return entry.values;
  }

  /** Asks for one generator's values, unless an answer is already on its way. */
  function warm(slot: SpecGeneratorSlot): void {
    const entry = cache.get(slot.key);
    if (entry !== undefined && entry.expiresAt > Date.now()) {
      return;
    }
    if (pending.has(slot.key)) {
      return;
    }

    pending.add(slot.key);
    void backend.completion
      .execute({
        handle: slot.handle,
        command: slot.command,
        args: [...slot.args],
        cwd: slot.cwd,
        timeoutMs: TIMEOUT_MS,
        maxOutputBytes: MAX_OUTPUT_BYTES,
      })
      .then((result) => {
        const values = result === null ? [] : parseValues(result.stdout, slot);
        remember(slot.key, values);
      })
      .catch(() => {
        remember(slot.key, []);
      })
      .finally(() => {
        pending.delete(slot.key);
      });
  }

  function remember(key: string, values: readonly SpecValueItem[]): void {
    cache.set(key, {
      expiresAt: Date.now() + (values.length > 0 ? VALUE_TTL_MS : EMPTY_TTL_MS),
      values,
    });
    version.value += 1;
  }

  /** A session that has ended has no generator answers left worth keeping. */
  function forget(handle: string): void {
    // A copy, because entries are deleted while they are walked.
    for (const key of Array.from(cache.keys())) {
      if (key.includes(`"${handle}"`)) {
        cache.delete(key);
      }
    }
  }

  return { version, cached, warm, forget };
});

/**
 * stdout as candidates.
 *
 * Generators are programs, not formatters: their output carries whitespace, a current-item marker
 * and sometimes a symbolic ref. The cleaning here is deliberately generic; the one command-specific
 * rule is `git branch`'s `remotes/` prefix, which is not a valid checkout target on its own.
 */
export function parseValues(
  stdout: string,
  slot: Pick<SpecGeneratorSlot, "command" | "args" | "splitOn">,
): SpecValueItem[] {
  const separator = slot.splitOn !== null && slot.splitOn !== "" ? slot.splitOn : "\n";
  const values: SpecValueItem[] = [];
  const seen = new Set<string>();
  const gitBranch = slot.command === "git" && slot.args.includes("branch");

  for (const raw of stdout.split(separator)) {
    if (values.length >= MAX_VALUES) {
      break;
    }
    let line = raw.replace(/\r$/, "");
    if (line.trim() === "") {
      continue;
    }

    let label = line;
    let detail: string | undefined;
    const tab = line.indexOf("\t");
    if (tab >= 0) {
      label = line.slice(0, tab);
      detail = line.slice(tab + 1).trim() || undefined;
    }

    label = label.trim();
    if (gitBranch) {
      if (label.includes(" -> ")) {
        continue;
      }
      label = label.replace(/^\*\s+/, "").replace(/^remotes\//, "");
    }
    if (label === "" || seen.has(label)) {
      continue;
    }
    seen.add(label);
    values.push(detail === undefined ? { label } : { label, detail });
  }

  return values;
}
