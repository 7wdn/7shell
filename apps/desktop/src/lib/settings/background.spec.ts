/**
 * The picture behind the window, as the numbers say it should be.
 *
 * The interesting half is the second one: the same colour with an alpha is the whole trick that makes
 * a picture visible behind a window whose surfaces were opaque, and it has to keep the notation the
 * token was written in — the palettes declare `oklch`, a hand-picked palette declares hex, and
 * converting between them changes the colour rather than the alpha.
 */
import { describe, expect, it } from "vitest";

import {
  WALLPAPER_SURFACE_TOKENS,
  backgroundImageStyle,
  surfaceAlpha,
  surfaceColorWithAlpha,
} from "@/lib/settings/background";

describe("painting the picture", () => {
  it("covers the window by default and keeps all of it when told to fit", () => {
    expect(backgroundImageStyle({ displayMode: "fill", blur: 0 }, "blob:one")).toMatchObject({
      backgroundImage: 'url("blob:one")',
      backgroundSize: "cover",
      backgroundRepeat: "no-repeat",
    });
    expect(backgroundImageStyle({ displayMode: "fit", blur: 0 }, "blob:one")).toMatchObject({
      backgroundSize: "contain",
      backgroundRepeat: "no-repeat",
    });
    expect(backgroundImageStyle({ displayMode: "stretch", blur: 0 }, "blob:one")).toMatchObject({
      backgroundSize: "100% 100%",
    });
    // Centred paints at the picture's own size: no `background-size` at all is what says so.
    const centred = backgroundImageStyle({ displayMode: "center", blur: 0 }, "blob:one");
    expect(centred.backgroundSize).toBeUndefined();
    expect(centred.backgroundRepeat).toBe("no-repeat");
    expect(backgroundImageStyle({ displayMode: "tile", blur: 0 }, "blob:one")).toMatchObject({
      backgroundRepeat: "repeat",
    });
  });

  it("draws nothing at all when there is no picture", () => {
    expect(backgroundImageStyle({ displayMode: "fill", blur: 4 }, null)).toEqual({});
  });

  it("blurs by enlarging, so the blurred edge does not show as a rim", () => {
    const blurred = backgroundImageStyle({ displayMode: "fill", blur: 6 }, "blob:one");
    expect(blurred.filter).toBe("blur(6px)");
    expect(blurred.transform).toBe("scale(1.06)");

    const sharp = backgroundImageStyle({ displayMode: "fill", blur: 0 }, "blob:one");
    expect(sharp.filter).toBeUndefined();
    expect(sharp.transform).toBeUndefined();
  });
});

describe("what the picture does to the surfaces", () => {
  it("turns the transparency into the alpha the surfaces are left with", () => {
    expect(surfaceAlpha(0)).toBe(1);
    expect(surfaceAlpha(20)).toBeCloseTo(0.8, 5);
    expect(surfaceAlpha(95)).toBeCloseTo(0.05, 5);
  });

  it("keeps a value past the slider's end inside it", () => {
    expect(surfaceAlpha(-10)).toBe(1);
    expect(surfaceAlpha(400)).toBeCloseTo(0.05, 5);
  });

  it("adds an alpha in the notation the colour was written in", () => {
    // The palettes write oklch: converting it to rgb to add an alpha would clip the wide-gamut ones.
    expect(surfaceColorWithAlpha("oklch(0.191 0.0043 264.5)", 0.8)).toBe("oklch(0.191 0.0043 264.5 / 0.8)");
    expect(surfaceColorWithAlpha("rgb(17 18 20)", 0.5)).toBe("rgb(17 18 20 / 0.5)");
    expect(surfaceColorWithAlpha("#101214", 0.5)).toBe("rgb(16 18 20 / 0.5)");
    expect(surfaceColorWithAlpha("#abc", 0.5)).toBe("rgb(170 187 204 / 0.5)");
    expect(surfaceColorWithAlpha("240 10% 4%", 0.5)).toBe("hsl(240 10% 4% / 0.5)");
  });

  it("answers nothing rather than a colour that is subtly wrong", () => {
    expect(surfaceColorWithAlpha("", 0.5)).toBeNull();
    expect(surfaceColorWithAlpha("not a colour", 0.5)).toBeNull();
    expect(surfaceColorWithAlpha("#12345", 0.5)).toBeNull();
  });

  it("names the surfaces a picture shows through, and the ones it does not", () => {
    // The frame and the panels: the picture is meant to be seen there. `--card` is deliberately not in
    // the list — the cards and dialogs carry the text, and the setting promises they stay readable.
    expect([...WALLPAPER_SURFACE_TOKENS]).toEqual(["--background", "--muted", "--accent"]);
  });
});
