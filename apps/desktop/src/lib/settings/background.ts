/**
 * The picture behind the window: how it is painted, and what it does to the surfaces in front of it.
 *
 * Two things live here and they are separate on purpose. The first is the paint style — five ways of
 * fitting a picture into a shape it does not match, which is a mapping from a word to CSS and nothing
 * else. The second is what the picture does to the window: while one is behind everything, the
 * surfaces *between* it and the work stop being opaque, or the picture would be invisible behind a
 * solid window. Which surfaces those are is the whole of the readability argument — the frame and the
 * panels let it through, the cards and dialogs that carry text do not.
 */
import { BACKGROUND_TRANSPARENCY_MAX, type BackgroundImage } from "@/types/app";

/**
 * How the picture is painted into the layer behind everything.
 *
 * `fill` covers and crops, `fit` keeps all of it and leaves bars, `stretch` distorts it, `center`
 * paints it at its own size, `tile` repeats it — the five the desktop has taught everybody, so that
 * none of them needs explaining in the interface.
 */
export function backgroundImageStyle(
  image: Pick<BackgroundImage, "displayMode" | "blur">,
  url: string | null,
): Record<string, string> {
  if (!url) {
    return {};
  }

  const style: Record<string, string> = { backgroundImage: `url("${url}")` };
  switch (image.displayMode) {
    case "fit":
      style.backgroundSize = "contain";
      style.backgroundPosition = "center";
      style.backgroundRepeat = "no-repeat";
      break;
    case "stretch":
      style.backgroundSize = "100% 100%";
      style.backgroundRepeat = "no-repeat";
      break;
    case "center":
      style.backgroundPosition = "center";
      style.backgroundRepeat = "no-repeat";
      break;
    case "tile":
      style.backgroundRepeat = "repeat";
      break;
    default:
      style.backgroundSize = "cover";
      style.backgroundPosition = "center";
      style.backgroundRepeat = "no-repeat";
      break;
  }

  if (image.blur > 0) {
    style.filter = `blur(${image.blur}px)`;
    // Enlarged a little, because a blurred edge is a soft edge: without this the layer's own edge
    // shows as a lighter rim against the window.
    style.transform = "scale(1.06)";
  }

  return style;
}

/** The alpha the surfaces take: the transparency is what the slider says, as a fraction that is left. */
export function surfaceAlpha(transparency: number): number {
  const clamped = Math.min(Math.max(Math.round(transparency), 0), BACKGROUND_TRANSPARENCY_MAX);
  return 1 - clamped / 100;
}

/**
 * The same colour, with an alpha, in the notation it was written in.
 *
 * The window's tokens are declared in several notations — the palettes write `oklch`, a hand-picked
 * palette writes `#rrggbb` — and converting them to rgb to add an alpha would clip the ones that only
 * exist outside it. So the notation is kept and the alpha is added inside it; anything unreadable
 * answers `null` rather than a colour that is subtly wrong.
 */
export function surfaceColorWithAlpha(color: string, alpha: number): string | null {
  const trimmed = color.trim();
  if (!trimmed) {
    return null;
  }

  const functional = /^([a-zA-Z-]+)\(([^()]+)\)$/.exec(trimmed);
  const name = functional?.[1];
  const body = functional?.[2];
  if (name && body) {
    const base = body.split("/")[0]?.trim() ?? "";
    if (!base) {
      return null;
    }
    return `${name}(${base} / ${alpha})`;
  }

  const hex = trimmed.startsWith("#") ? trimmed.slice(1) : null;
  if (hex && (hex.length === 3 || hex.length === 6)) {
    const expanded =
      hex.length === 3
        ? hex
            .split("")
            .map((digit) => digit + digit)
            .join("")
        : hex;
    const [red, green, blue] = [0, 2, 4].map((offset) => Number.parseInt(expanded.slice(offset, offset + 2), 16));
    if ([red, green, blue].some((channel) => Number.isNaN(channel))) {
      return null;
    }
    return `rgb(${red} ${green} ${blue} / ${alpha})`;
  }

  // A bare channel triplet, which is how a hand-written theme may spell hsl channels.
  if (/^[\d.]+%?\s+[\d.]+%?\s+[\d.]+%?$/.test(trimmed)) {
    return `hsl(${trimmed} / ${alpha})`;
  }

  return null;
}

/**
 * The surfaces a picture shows through.
 *
 * The frame and the panels, and nothing that carries text on a surface of its own: the cards, the
 * dialogs and the popovers keep their colour whatever is behind them, which is what the setting's own
 * words promise. `--background` is the window, `--muted` the quieter blocks on it and `--accent` the
 * lit rows — each of them a place where the picture is meant to be seen rather than read through.
 */
export const WALLPAPER_SURFACE_TOKENS = ["--background", "--muted", "--accent"] as const;

/**
 * Makes those surfaces translucent, and answers with the way back.
 *
 * The originals are read back from the document rather than recomputed, because the palette in force
 * is the stylesheet's business: whatever colour the window is wearing right now is what gets an alpha
 * put on top of it. The answer is a function rather than a module-level toggle so that the caller owns
 * the lifetime — a picture that goes away must put the surfaces back exactly as they were.
 */
export function withTranslucentSurfaces(transparency: number): () => void {
  const root = document.documentElement;
  const alpha = surfaceAlpha(transparency);
  const previous = WALLPAPER_SURFACE_TOKENS.map((token) => [token, root.style.getPropertyValue(token)] as const);

  for (const token of WALLPAPER_SURFACE_TOKENS) {
    const current = getComputedStyle(root).getPropertyValue(token);
    const translucent = surfaceColorWithAlpha(current, alpha);
    if (translucent) {
      root.style.setProperty(token, translucent);
    }
  }

  return () => {
    for (const [token, value] of previous) {
      if (value) {
        root.style.setProperty(token, value);
      } else {
        root.style.removeProperty(token);
      }
    }
  };
}
