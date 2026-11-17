// Converts CSS lab() colours to sRGB, for the browser package's colour fallbacks.
// Packaging-only: the desktop build keeps its wide-gamut values. This file is a library — the
// conversion itself lives in legacy-css.cjs — so requiring it must do nothing on its own.

function labToSrgb(L, a, b) {
  const fy = (L + 16) / 116;
  const fx = fy + a / 500;
  const fz = fy - b / 200;
  const finv = (t) => {
    const t3 = t * t * t;
    return t3 > 0.008856451679035631 ? t3 : (116 * t - 16) / 903.2962962962963;
  };
  const x50 = 0.96422 * finv(fx);
  const y50 = finv(fy);
  const z50 = 0.82521 * finv(fz);
  const x = 0.9554734527042182 * x50 - 0.023098536874261423 * y50 + 0.0632593086610217 * z50;
  const y = -0.028369706963208136 * x50 + 1.0099954580058226 * y50 + 0.021041398966943008 * z50;
  const z = 0.012314001688319899 * x50 - 0.020507696433477912 * y50 + 1.3303659366080753 * z50;
  const encode = (u) => {
    const c = Math.max(0, Math.min(1, u));
    return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
  };
  const r = 3.2409699419045226 * x - 1.537383177570094 * y - 0.4986107602930034 * z;
  const g = -0.9692436362808796 * x + 1.8759675015077202 * y + 0.04155505740717559 * z;
  const bl = 0.05563007969699366 * x - 0.20397695888897652 * y + 1.0569715142428786 * z;
  return [Math.round(encode(r) * 255), Math.round(encode(g) * 255), Math.round(encode(bl) * 255)];
}

const labPattern = /lab\(\s*([+-]?[\d.]+)%\s+([+-]?[\d.]+)\s+([+-]?[\d.]+)\s*(?:\/\s*([\d.]+)%?)?\s*\)/g;

function labToCss(match, L, a, b, alpha) {
  const [r, g, bl] = labToSrgb(Number(L), Number(a), Number(b));
  if (alpha === undefined) return `rgb(${r}, ${g}, ${bl})`;
  const value = Number(alpha) > 1 ? Number(alpha) / 100 : Number(alpha);
  return `rgba(${r}, ${g}, ${bl}, ${Number(value.toFixed(3))})`;
}

module.exports = { labToSrgb, labToCss, labPattern };
