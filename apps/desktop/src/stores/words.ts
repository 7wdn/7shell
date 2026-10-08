/**
 * The words somebody added to what follows a command.
 *
 * Kept apart from the command library because they are different things: a library entry is a whole
 * line somebody sends, and this is a word the command line offers while somebody types. They are
 * read together — the completion asks for both — and they are edited in different places.
 *
 * A word has no place of its own in the order: the order it was added in is the whole of it, and all
 * it decides is which of two equally good words is offered first.
 */
import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import type { CompletionWord } from "@/types/snippet";

/** The fields a word is written from, before it is stored. */
export interface WordDraft {
  id: string;
  command: string;
  word: string;
  note: string;
}

export const useWordsStore = defineStore("words", () => {
  const words = ref<CompletionWord[]>([]);
  const loaded = ref(false);
  const failure = ref<ShellError | null>(null);

  async function load(): Promise<void> {
    try {
      words.value = await backend.commands.words();
      failure.value = null;
      loaded.value = true;
    } catch (error) {
      // The library's code, because it is the same file that could not be read: the words are stored
      // beside the commands, and "the library could not be opened" is what happened.
      failure.value = asShellError(error, "snippets_open");
    }
  }

  /**
   * Adds a word, or stores what an edit made of one.
   *
   * Answers whether it was written, which is what lets a dialog stay open on the word that did not
   * land rather than closing over a failure.
   */
  async function save(draft: WordDraft): Promise<boolean> {
    if (draft.command.trim() === "" || draft.word.trim() === "") {
      return false;
    }

    try {
      await backend.commands.saveWord({
        id: draft.id,
        command: draft.command.trim(),
        word: draft.word.trim(),
        // The note is the only field kept as it was typed: it is a sentence somebody wrote, and a
        // trailing space in it is theirs.
        note: draft.note.trim(),
      });
      await load();
      failure.value = null;
      return true;
    } catch (error) {
      failure.value = asShellError(error, "snippet_save");
      return false;
    }
  }

  async function remove(id: string): Promise<void> {
    try {
      await backend.commands.deleteWord(id);
      await load();
    } catch (error) {
      failure.value = asShellError(error, "snippet_delete");
    }
  }

  return { words, loaded, failure, load, save, remove };
});
