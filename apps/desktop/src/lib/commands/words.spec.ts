/**
 * The two halves of "what comes after this command", joined.
 *
 * These are the rules the command line reads: what somebody added first, each word once, and their
 * own line about a word rather than ours. They are tested here rather than through the menu because
 * the menu cannot say *why* a word is where it is — and because a rule about the order is the kind
 * of thing that goes missing in a refactor without anything else failing.
 */
import { describe, expect, it } from "vitest";

import { noteFor, wordsFor } from "@/lib/commands/words";
import type { CompletionWord } from "@/types/snippet";

function added(command: string, word: string, note = ""): CompletionWord {
  return { id: `word-${command}-${word}`, command, word, note };
}

describe("the words that come after a command", () => {
  it("puts what somebody added before what this build ships", () => {
    const words = wordsFor("git", [added("git", "merged")], ["status", "commit"]);

    expect(words).toEqual(["merged", "status", "commit"]);
  });

  it("keeps the order they were added in", () => {
    const words = wordsFor("kubectl", [added("kubectl", "drain"), added("kubectl", "apply")], []);

    expect(words).toEqual(["drain", "apply"]);
  });

  it("has a word once, even when we ship the same one", () => {
    const words = wordsFor("kubectl", [added("kubectl", "apply")], ["apply", "get", "apply"]);

    expect(words).toEqual(["apply", "get"]);
  });

  it("answers for a command this build has never heard of", () => {
    // The whole reason the list exists: an in-house tool is in nobody's table but its owner's.
    const words = wordsFor("acme-deploy", [added("acme-deploy", "drain")], []);

    expect(words).toEqual(["drain"]);
  });

  it("says nothing about a command nobody added anything for", () => {
    expect(wordsFor("rsync", [added("kubectl", "drain")], ["-av"])).toEqual(["-av"]);
  });
});

describe("what a word means", () => {
  it("prefers the line somebody wrote themselves", () => {
    // Their sentence is about what *they* mean by the word; ours is about how we use it, which is a
    // different answer to a question they have already answered.
    expect(noteFor("kubectl", "apply", [added("kubectl", "apply", "收起那份清单")])).toBe("收起那份清单");
  });

  it("has nothing for a word somebody added without a line about it", () => {
    expect(noteFor("kubectl", "apply", [added("kubectl", "apply", "   ")])).toBeNull();
  });

  it("has nothing for a word nobody added", () => {
    expect(noteFor("git", "status", [added("kubectl", "apply", "收起那份清单")])).toBeNull();
  });

  it("matches the command as well as the word", () => {
    // `apply` means one thing after `kubectl` and could mean another after something else, so a note
    // belongs to the pair rather than to the word.
    const words = [added("kubectl", "apply", "kubectl"), added("acme", "apply", "acme")];

    expect(noteFor("acme", "apply", words)).toBe("acme");
  });
});
