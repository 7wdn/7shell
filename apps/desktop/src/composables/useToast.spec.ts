/**
 * The transient slot: one message at a time, and a clock that belongs to the tier.
 *
 * The durations are the part worth pinning down: they are what the person actually experiences, and
 * a tier that quietly stops meaning what it says would only be noticed by somebody staring at the
 * window, waiting for a message to leave.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TOAST_TIERS, useToast } from "@/composables/useToast";

const { message, tier, action, visible, toast } = useToast();

describe("the toast slot", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    visible.value = false;
    action.value = undefined;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows one message and marks it visible", () => {
    toast("Copied", "receipt");

    expect(message.value).toBe("Copied");
    expect(tier.value).toBe("receipt");
    expect(visible.value).toBe(true);
  });

  it("replaces the message on screen instead of stacking a second one", () => {
    toast("first", "receipt");
    toast("second", "confirmation");

    expect(message.value).toBe("second");
    expect(tier.value).toBe("confirmation");
  });

  it("keeps a receipt for exactly as long as a receipt is worth", () => {
    toast("Copied", "receipt");

    vi.advanceTimersByTime(TOAST_TIERS.receipt - 1);
    expect(visible.value).toBe(true);

    vi.advanceTimersByTime(1);
    expect(visible.value).toBe(false);
  });

  it("keeps a failure long enough to be read", () => {
    toast("Upload failed", "failure");

    vi.advanceTimersByTime(TOAST_TIERS.receipt);
    expect(visible.value).toBe(true);

    vi.advanceTimersByTime(TOAST_TIERS.failure - TOAST_TIERS.receipt);
    expect(visible.value).toBe(false);
  });

  it("restarts the clock when a later message takes the slot", () => {
    toast("Finished while you were away", "background");
    vi.advanceTimersByTime(TOAST_TIERS.background - 500);

    toast("Copied", "receipt");

    // The replaced message's timer must not take the new one down with it.
    vi.advanceTimersByTime(TOAST_TIERS.receipt - 1);
    expect(visible.value).toBe(true);
    vi.advanceTimersByTime(1);
    expect(visible.value).toBe(false);
  });

  it("drops the action together with the message it belonged to", () => {
    const onClick = vi.fn();
    toast("Batch finished", "background", { label: "Show results", onClick });

    vi.advanceTimersByTime(TOAST_TIERS.background);

    expect(visible.value).toBe(false);
    expect(action.value).toBeUndefined();
  });

  it("replaces the action too, so a stale button cannot survive a new message", () => {
    const first = vi.fn();
    const second = vi.fn();
    toast("Batch finished", "background", { label: "Show results", onClick: first });
    toast("Another batch finished", "background", { label: "Show results", onClick: second });

    action.value?.onClick();

    expect(second).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();
  });
});
