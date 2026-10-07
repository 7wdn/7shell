/**
 * The five colours somebody picks, turned into the tokens a window is drawn from.
 *
 * A window is made of about a dozen colours and a person is going to pick five, so something has to
 * decide the rest. It is decided here, once, rather than by whoever writes the next component:
 *
 * 1. **The surfaces move together.** Every panel colour is the background nudged a few percent
 *    toward the foreground, which is what makes a card look raised in a light theme and sunk in a
 *    dark one without anybody choosing a direction — the same rule works both ways round, and it
 *    keeps working when somebody picks a light foreground on a dark background or the reverse.
 * 2. **Text is never picked, it is derived.** Muted text is the foreground pulled most of the way
 *    back toward the background, and the colour written *on* the accent is whichever of white and
 *    near-black reads better against it (WCAG contrast, not a guess). That is the whole reason the
 *    five are five: a person choosing colours cannot arrive at a combination nobody can read.
 * 3. **The terminal is dressed from the same five**, so a hand-picked palette does not leave the
 *    one pane that is always on screen in the build's own greys.
 *
 * The ratios are the reference's, with one exception. Its muted text is pulled 58% of the way back
 * toward the page, which on the pale pair this starts from reads at 2.9:1 — under the 3:1 floor a
 * piece of text has to clear to be text at all, and the one number here that has a right answer
 * rather than a taste. Halfway is 3.9:1 on the same pair, and still visibly quieter than the
 * foreground; `customColors.spec.ts` holds the floor rather than the ratio.
 */
import type { CustomColors } from "@/types/app";

/** Every colour is written out as `rgb(r g b)`, which is what a token holding a hex has to become. */
function rgb(hex: string): string {
  const value = hex.replace("#", "");
  const r = parseInt(value.slice(0, 2), 16);
  const g = parseInt(value.slice(2, 4), 16);
  const b = parseInt(value.slice(4, 6), 16);
  return `rgb(${r} ${g} ${b})`;
}

function channels(hex: string): [number, number, number] {
  const value = hex.replace("#", "");
  return [parseInt(value.slice(0, 2), 16), parseInt(value.slice(2, 4), 16), parseInt(value.slice(4, 6), 16)];
}

/** `weight` of the way from `a` to `b`, which is what every derived surface is. */
export function mix(a: string, b: string, weight: number): string {
  const [ar, ag, ab] = channels(a);
  const [br, bg, bb] = channels(b);
  const blend = (x: number, y: number): number => Math.round(x + (y - x) * weight);
  return `rgb(${blend(ar, br)} ${blend(ag, bg)} ${blend(ab, bb)})`;
}

/** WCAG 2.x relative luminance, 0 (black) to 1 (white). */
function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** White or near-black, whichever reads better on `hex` — so the accent never carries invisible text. */
export function readableTextOn(hex: string): string {
  const on = (candidate: string): number => {
    const [a, b] = [luminance(hex), luminance(candidate)].sort((x, y) => y - x) as [number, number];
    return (a + 0.05) / (b + 0.05);
  };
  return on("#ffffff") >= on("#0a0a0a") ? "#ffffff" : "#0a0a0a";
}

/**
 * The tokens one set of five colours is worth, as CSS custom properties.
 *
 * Returned as a map rather than applied, because the two sets — light and dark — are both needed at
 * once: the terminal draws in whichever of the two it was told to, which is not always the one the
 * window around it is in.
 *
 * @param theme which of the pane's two palettes this set is being derived for; only the terminal's
 * selection differs, and it differs for the reason given at `deriveFor`.
 */
export function deriveCustomColors(colors: CustomColors, theme: "light" | "dark"): Record<string, string> {
  const { background, foreground, primary, border, card } = colors;
  const accent = mix(background, foreground, 0.12);
  const muted = mix(background, foreground, 0.06);

  // The value written on the accent is picked by contrast, and the selection inside the terminal is
  // the palette's own tint of the page: the accent in the dark set, which is dark enough to read
  // through, and the input one step stronger in the light set, whose accent is nearly the page.
  const selection = theme === "dark" ? accent : mix(background, foreground, 0.16);

  return {
    "--background": rgb(background),
    "--foreground": rgb(foreground),
    "--card": rgb(card),
    "--card-foreground": rgb(foreground),
    "--muted": muted,
    "--muted-foreground": mix(foreground, background, 0.5),
    "--border": rgb(border),
    "--input": rgb(border),
    "--primary": rgb(primary),
    "--primary-foreground": rgb(readableTextOn(primary)),
    "--accent": accent,
    "--accent-foreground": rgb(foreground),
    "--ring": rgb(primary),
    [`--terminal-${theme}-background`]: rgb(card),
    [`--terminal-${theme}-foreground`]: rgb(foreground),
    [`--terminal-${theme}-cursor`]: rgb(primary),
    [`--terminal-${theme}-selection`]: selection,
  };
}
