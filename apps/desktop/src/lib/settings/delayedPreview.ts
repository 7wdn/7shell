/**
 * A preview that waits a moment before it runs.
 *
 * Pointing at a list of colours is a sweep: the pointer crosses four options on its way to the one
 * somebody is aiming at, and showing each of them as it passes is a window strobed four times. The
 * delay is what makes the list calm to use — long enough that crossing an option is not choosing it,
 * short enough that coming to rest on one still reads as immediate.
 *
 * There is no immediate path here, because there is only one thing that triggers a preview: the
 * pointer. An arrow key moves the list's highlight rather than its focus, which no event reports —
 * see the note on the dropdown's `option-hover` — so a "show it now" entry point would be one
 * nothing could call.
 */
export interface DelayedPreview<T> {
  /** Runs `apply` with `value` once the delay has passed, unless something else is scheduled. */
  schedule(value: T): void;
  /** Forgets what was scheduled, so it never runs. */
  cancel(): void;
}

export function createDelayedPreview<T>(apply: (value: T) => void, delayMs: number): DelayedPreview<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;

  function cancel(): void {
    if (timer === undefined) {
      return;
    }
    clearTimeout(timer);
    timer = undefined;
  }

  return {
    schedule(value: T): void {
      cancel();
      timer = setTimeout(() => {
        timer = undefined;
        apply(value);
      }, delayMs);
    },
    cancel,
  };
}
