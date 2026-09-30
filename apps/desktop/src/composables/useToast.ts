/**
 * The transient surface: one line at the foot of the window that leaves on its own.
 *
 * What belongs here is decided by one question — is this message about a *place*, or about an
 * *action*?
 *
 * - About a place (a field, a tab, a session, a panel): it stays in that place. The location is its
 *   context, it has to survive until the person acts on it, and an envelope they can quote back is
 *   worth more than anything that disappears. A refused unlock, a failed listing, a failed save:
 *   all of these are about where they happened.
 * - About an action whose context is gone — a receipt for something that left no visible trace, or
 *   a failure of work that was started from a panel that is no longer on screen — there is nowhere
 *   for it to live in the page. That is what this surface is for.
 * - About work that keeps running: the task surface owns it (the transfer list). A toast can be a
 *   pointer to it, never the record and never the progress bar.
 *
 * Two rules follow, and both matter more than the styling:
 *
 * 1. **One message has exactly one home.** Nothing is ever said here *and* written into a page.
 * 2. **The gate screen stays silent.** While the vault is shut there is nothing else to do, so its
 *    messages stay on the page: a hint that expires while somebody is still deciding is worse than
 *    no hint at all.
 *
 * A message that describes a failure is always the output of `describeShellError`, so what appears
 * here is the same sentence the page would have shown, in the same language, from the same envelope.
 *
 * Durations are tiers rather than numbers. How long a person needs to read something is a decision
 * about the message, not about the call site, and making that decision once is the only way the
 * ladder stays consistent.
 */
import { ref, type Ref } from "vue";

/** How long each kind of message stays; the tier is the decision, the number is bookkeeping. */
export const TOAST_TIERS = {
  /** The action left no visible trace of its own: a copy, a refresh, a value echoed back. */
  receipt: 1500,
  /** The default: something was saved, opened, applied, and the result is on screen. */
  confirmation: 2000,
  /** Worth reading before it goes: a name, a count, which row was affected. */
  result: 2500,
  /** Refused, and the reason has to be read to correct what was typed. */
  refusal: 4000,
  /** Something failed. */
  failure: 5000,
  /** Finished while the person was elsewhere, or finished partly: the whole line needs reading. */
  background: 8000,
  /**
   * Something failed while the person was looking elsewhere — a transfer they started and walked
   * away from. This is the one tier that is both long and interrupting: nothing else will tell
   * them, and they are in the middle of something else when it arrives.
   */
  unseenFailure: 8000,
} as const;

export type ToastTier = keyof typeof TOAST_TIERS;

/** Whether a tier is worth interrupting for. The viewport draws these in its assertive region. */
export function toastInterrupts(tier: ToastTier): boolean {
  return tier === "refusal" || tier === "failure" || tier === "unseenFailure";
}

/** An optional button on the toast: the one way back to whatever it is talking about. */
export interface ToastAction {
  readonly label: string;
  readonly onClick: () => void;
}

interface ToastState {
  message: Ref<string>;
  tier: Ref<ToastTier>;
  action: Ref<ToastAction | undefined>;
  visible: Ref<boolean>;
  timer: number;
}

/**
 * One slot for the whole window, shared by every caller. There is deliberately no queue: a second
 * message means the first one is out of date, and replacing it is the honest thing to do.
 */
const state: ToastState = {
  message: ref(""),
  tier: ref("confirmation"),
  action: ref(undefined),
  visible: ref(false),
  timer: 0,
};

export function useToast() {
  function toast(message: string, tier: ToastTier = "confirmation", action?: ToastAction): void {
    state.message.value = message;
    state.tier.value = tier;
    state.action.value = action;
    state.visible.value = true;

    // Replacing a message restarts the clock, so whatever is on screen always gets the full time
    // its own tier asks for. There is no way to dismiss a toast early: a message either expires or
    // is superseded, which keeps "gone" from ever meaning "somebody clicked it away".
    clearTimeout(state.timer);
    state.timer = window.setTimeout(() => {
      state.visible.value = false;
      state.action.value = undefined;
    }, TOAST_TIERS[tier]);
  }

  return { message: state.message, tier: state.tier, action: state.action, visible: state.visible, toast };
}
