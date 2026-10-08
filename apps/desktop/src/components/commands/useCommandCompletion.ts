/**
 * The state under the completion list: what is offered, which row the keyboard is on, and when to
 * stay shut.
 *
 * The engine (`lib/commands/completion.ts`) is a pure function of the line; this is the little bit of
 * state a menu needs on top of it, and nothing about the drawing. The line belongs to the caller —
 * the component owns the input — so `take` answers with a new line and where the caret goes, and the
 * caller writes both.
 */
import { computed, ref, type ComputedRef, type Ref } from "vue";

import type { CompletionCandidate } from "@/lib/commands/completion";
import { completionSiteAt } from "@/lib/commands/tokenize";

/** Builds the list for a line as it now stands. Injected so this file needs no store. */
export type CompletionBuilder = (line: string, caret: number) => readonly CompletionCandidate[];

export interface CommandCompletion {
  readonly candidates: Ref<readonly CompletionCandidate[]>;
  readonly active: Ref<number>;
  readonly visible: ComputedRef<boolean>;
  /**
   * True once somebody has pointed at a row with the arrow keys.
   *
   * The list opens with its first row under the keyboard — that is the row the accept key takes — but
   * nothing is *chosen* until somebody says so. It is the difference between "the list is in the way"
   * and "I mean this one": running a line on Enter is the bar's whole job, and a row nobody pointed at
   * must not take that over.
   */
  readonly chosen: Ref<boolean>;
  /** Rebuild for the line as it stands, unless this exact line was refused. */
  refresh(caret: number): void;
  /** Open on purpose — Tab, the trigger key — whatever was refused before. */
  open(caret: number): void;
  /** Take the list away without refusing anything: a composition started, or the field lost focus. */
  hide(): void;
  /** Escape: shut for this line until it changes. */
  dismiss(caret: number): void;
  move(delta: number): void;
  /** What taking a row does to the line, or `null` when that row is not there. */
  take(caret: number): { text: string; caret: number } | null;
}

export function useCommandCompletion(line: Ref<string>, build: CompletionBuilder): CommandCompletion {
  const candidates = ref<readonly CompletionCandidate[]>([]);
  const active = ref(0);
  const chosen = ref(false);
  const visible = computed(() => candidates.value.length > 0);

  /**
   * The line somebody pressed Escape on.
   *
   * A refusal lasts as long as that line does and no longer: typing one more character is a new
   * question, and the answer to it arrives then. A timer would either reopen under the hands of
   * somebody still typing or keep the list away when they asked again.
   */
  let refused: string | null = null;

  function fingerprint(caret: number): string {
    return `${caret}@${line.value}`;
  }

  function show(list: readonly CompletionCandidate[]): void {
    candidates.value = list;
    active.value = 0;
    chosen.value = false;
  }

  function hide(): void {
    candidates.value = [];
    active.value = 0;
    chosen.value = false;
  }

  function refresh(caret: number): void {
    if (refused !== null && refused === fingerprint(caret)) {
      hide();
      return;
    }

    // An empty word is answered when somebody asks for it — the accept key on its own, the trigger
    // key, the step after taking a command — and not while they are typing past it. A list that
    // appeared every time a space was typed would be a list in the way of the next word.
    if (completionSiteAt(line.value, caret).prefix === "") {
      hide();
      return;
    }

    show(build(line.value, caret));
  }

  function open(caret: number): void {
    refused = null;
    show(build(line.value, caret));
  }

  function dismiss(caret: number): void {
    refused = fingerprint(caret);
    hide();
  }

  function move(delta: number): void {
    const count = candidates.value.length;
    if (count === 0) {
      return;
    }

    // Nothing chosen yet: the first press lands on the first row going down and the last going up,
    // which is what a list that has not been pointed at looks like it should do.
    const from = chosen.value ? active.value : delta > 0 ? -1 : 0;
    active.value = (from + delta + count) % count;
    chosen.value = true;
  }

  function take(caret: number): { text: string; caret: number } | null {
    const candidate = candidates.value[active.value];
    if (candidate === undefined) {
      return null;
    }

    const rest = line.value.slice(caret);
    // A word the candidate leaves room for gets a space after it; a whole line is the sentence, and a
    // directory is half a path. Either way a space somebody has already typed is not doubled.
    const tail = candidate.trailingSpace && rest === "" ? " " : "";
    const text = `${line.value.slice(0, candidate.from)}${candidate.insert}${tail}${rest}`;
    const at = candidate.from + candidate.insert.length + tail.length;

    hide();
    // Taking a row is a question this line already answered, so the list does not come back for it.
    refused = `${at}@${text}`;
    return { text, caret: at };
  }

  return { candidates, active, chosen, visible, refresh, open, hide, dismiss, move, take };
}
