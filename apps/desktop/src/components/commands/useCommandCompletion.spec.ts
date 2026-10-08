import { ref } from "vue";
import { describe, expect, it } from "vitest";

import { useCommandCompletion } from "@/components/commands/useCommandCompletion";
import type { CompletionCandidate } from "@/lib/commands/completion";

function candidate(over: Partial<CompletionCandidate> = {}): CompletionCandidate {
  return {
    label: "uptime",
    detail: null,
    insert: "uptime",
    kind: "command",
    from: 0,
    score: 1,
    wholeLine: false,
    trailingSpace: true,
    labelRanges: [],
    detailRanges: [],
    ...over,
  };
}

/** A completion holding one candidate, opened at `caret`. */
function open(line: string, caret: number, over: Partial<CompletionCandidate> = {}) {
  const text = ref(line);
  const completion = useCommandCompletion(text, () => [candidate(over)]);
  completion.open(caret);
  return { text, completion, caret };
}

describe("taking the row the keyboard is on", () => {
  it("leaves a space after a word, which is where the next one goes", () => {
    const { completion, caret } = open("upt", 3);

    expect(completion.take(caret)).toEqual({ text: "uptime ", caret: 7 });
  });

  it("leaves nothing after a whole line, which is already the sentence", () => {
    const { completion, caret } = open("upt", 3, { insert: "uptime -p", wholeLine: true, trailingSpace: false });

    expect(completion.take(caret)).toEqual({ text: "uptime -p", caret: 9 });
  });

  it("leaves a directory as half a path, with the caret where the next keystroke continues", () => {
    const { completion, caret } = open("cd /tm", 6, { from: 3, insert: "/tmp/", trailingSpace: false });

    expect(completion.take(caret)).toEqual({ text: "cd /tmp/", caret: 8 });
  });

  it("does not put a space next to one somebody already typed", () => {
    const { completion, caret } = open("uptime - x", 8, { from: 7, insert: "-p" });

    expect(completion.take(caret)).toEqual({ text: "uptime -p x", caret: 9 });
  });

  it("closes the list and answers nothing when there is no row to take", () => {
    const text = ref("upt");
    const completion = useCommandCompletion(text, () => []);

    expect(completion.take(3)).toBeNull();
  });

  it("stays shut for the line it was dismissed on, and opens for the next one", () => {
    const text = ref("upt");
    const completion = useCommandCompletion(text, () => [candidate()]);

    completion.refresh(3);
    expect(completion.visible.value).toBe(true);

    completion.dismiss(3);
    expect(completion.visible.value).toBe(false);

    completion.refresh(3);
    expect(completion.visible.value, "the same line again is the same question").toBe(false);

    text.value = "upti";
    completion.refresh(4);
    expect(completion.visible.value).toBe(true);
  });

  it("counts only a row somebody pointed at as chosen", () => {
    const text = ref("upt");
    const completion = useCommandCompletion(text, () => [candidate()]);

    completion.open(3);
    expect(completion.chosen.value, "the list is open but nothing is pointed at").toBe(false);

    completion.move(1);
    expect(completion.chosen.value).toBe(true);

    completion.hide();
    expect(completion.chosen.value, "the next list starts unchosen again").toBe(false);
  });

  it("answers nothing while the word is empty, however the list was asked for", () => {
    const text = ref("uptime ");
    let built = 0;
    const completion = useCommandCompletion(text, () => {
      built += 1;
      return [candidate()];
    });

    // Typing past a word: the refresh is what happens on every keystroke, and an empty word is not a
    // question to answer unasked.
    completion.refresh(7);
    expect(completion.visible.value).toBe(false);
    expect(built).toBe(0);

    // Asked for on purpose — Tab, the trigger key, the step after taking a command — it is answered.
    completion.open(7);
    expect(completion.visible.value).toBe(true);
    expect(built).toBe(1);
  });
});
