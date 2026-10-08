/**
 * The words somebody added, joined with the words this build ships.
 *
 * Both halves answer the same question — "what comes after this command" — so the two are one list
 * as far as the command line is concerned, and this is where they become one. Three rules, and each
 * of them is about what somebody typing sees:
 *
 * * **Theirs come first.** The tie-break in the ranking is the word's place in the list, so the order
 *   here is what decides which of two equally good words is offered first — and what somebody added
 *   for their own tool is a better guess than what ships with the build. It is the same rule the
 *   library has: what somebody made comes before what we ship.
 * * **A word appears once.** Somebody who adds `apply` for `kubectl` does not get two rows saying it
 *   because the build also ships it. The shipped one stays where it is in the list and the added one
 *   takes its place, which is what "theirs first" would have done anyway.
 * * **Their note wins over ours.** A word somebody wrote a line about says what *they* mean by it;
 *   the shipped line is about how we use the same word, and it is the wrong answer to a question
 *   somebody already answered themselves.
 */
import type { CompletionWord } from "@/types/snippet";

/**
 * What follows `command`: the words somebody added, then the ones this build ships.
 *
 * A command the shipped table has never heard of still answers with whatever was added for it, which
 * is the whole reason the list exists: an in-house tool is not in anybody's table but its owner's.
 */
export function wordsFor(command: string, added: readonly CompletionWord[], shipped: readonly string[]): string[] {
  const mine = added.filter((entry) => entry.command === command).map((entry) => entry.word);
  const seen = new Set(mine);
  return [...mine, ...shipped.filter((word) => !seen.has(word))];
}

/**
 * What a word means: their own line about it, or nothing.
 *
 * `null` rather than an empty string so the caller can tell "nobody wrote one" from "somebody wrote
 * nothing": the first falls back to what kind of word it is, and the second is the same thing, but
 * only one of them is the caller's decision to make.
 */
export function noteFor(command: string, word: string, added: readonly CompletionWord[]): string | null {
  const entry = added.find((candidate) => candidate.command === command && candidate.word === word);
  const note = entry?.note.trim() ?? "";
  return note === "" ? null : note;
}
