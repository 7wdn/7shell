/**
 * The preview that waits, and the way it is let go of.
 *
 * What it is for is the list of colours: the pointer crosses several on its way to the one somebody
 * is aiming at, and the window must show the one it comes to rest on rather than every one it
 * passes. Neither behaviour is visible in a screenshot — a schedule that did not cancel would show
 * every option crossed, a moment after the one that was landed on — and both would read as a
 * repainting glitch rather than as a bug.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createDelayedPreview } from "@/lib/settings/delayedPreview";

describe("a preview that waits", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("shows what the pointer came to rest on, not what it crossed", () => {
    const shown: string[] = [];
    const preview = createDelayedPreview<string>((value) => shown.push(value), 80);

    preview.schedule("mist");
    vi.advanceTimersByTime(20);
    preview.schedule("graphite");
    vi.advanceTimersByTime(20);
    preview.schedule("cobalt");
    vi.advanceTimersByTime(79);
    expect(shown, "nothing is shown while the pointer is still moving").toEqual([]);

    vi.advanceTimersByTime(1);
    expect(shown, "the one it stopped on, and only that one").toEqual(["cobalt"]);
  });

  it("shows the one before last when the pointer stops on two in a row", () => {
    const shown: string[] = [];
    const preview = createDelayedPreview<string>((value) => shown.push(value), 80);

    // Coming to rest on one option and then on the next is two previews, not one: the second only
    // replaces the first once it has been rested on as well.
    preview.schedule("sage");
    vi.advanceTimersByTime(80);
    preview.schedule("amber");
    vi.advanceTimersByTime(80);

    expect(shown).toEqual(["sage", "amber"]);
  });

  it("forgets what was scheduled when it is cancelled", () => {
    const shown: string[] = [];
    const preview = createDelayedPreview<string>((value) => shown.push(value), 80);

    preview.schedule("blush");
    preview.cancel();
    vi.advanceTimersByTime(500);

    expect(shown, "a cancelled preview never runs").toEqual([]);
  });
});
