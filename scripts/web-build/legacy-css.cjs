// Rewrites the compiled interface CSS for browsers without CSS Color 4 (UOS 20's stock browser).
// lab() becomes sRGB, and every color-mix() gets an equivalent rgba() declaration in front of it,
// so modern browsers keep the mix and older ones fall back to an alpha-scaled solid colour.
const fs = require("fs");
const path = require("path");
const { labToCss, labPattern } = require("./lab-check.cjs");

function transform(input) {
  let css = fs.readFileSync(input, "utf8");
  const before = { lab: (css.match(/lab\(/g) || []).length, mix: (css.match(/color-mix\(/g) || []).length };
  css = css.replace(labPattern, labToCss);

  const tokens = new Set();
  for (const m of css.matchAll(/color-mix\(in ok(?:lab|lch), var\((--[a-z0-9-]+)\)/g)) tokens.add(m[1]);
  for (const m of css.matchAll(/color-mix\(in oklch, var\((--[a-z0-9-]+)\)[^)]*var\((--[a-z0-9-]+)\)\)/g)) {
    tokens.add(m[1]);
    tokens.add(m[2]);
  }

  let companions = 0;
  css = css.replace(/(--[a-z0-9-]+)\s*:\s*(rgba?\([^;}]*\))/g, (whole, name, value) => {
    if (!tokens.has(name)) return whole;
    const m = /rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)(?:\s*,\s*([\d.]+))?\s*\)/.exec(value);
    if (!m) return whole;
    companions += 1;
    return `${name}:${value};${name}-rgb:${m[1]}, ${m[2]}, ${m[3]};${name}-a:${m[4] ?? "1"}`;
  });

  let chains = 0;
  css = css.replace(/(--[a-z0-9-]+)\s*:\s*var\((--[a-z0-9-]+)\)/g, (whole, name, target) => {
    if (!tokens.has(name)) return whole;
    chains += 1;
    return `${whole};${name}-rgb:var(${target}-rgb);${name}-a:var(${target}-a, 1)`;
  });

  let fallbacks = 0;
  const skipped = [];
  css = css.replace(
    /((?:--)?[-a-zA-Z][-a-zA-Z0-9]*)\s*:\s*(color-mix\(in ok(?:lab|lch),[^;{}]*\)(?:\s+transparent)?)/g,
    (whole, prop, value) => {
      const single = /^color-mix\(in ok(?:lab|lch), var\((--[a-z0-9-]+)\)\s+([\d.]+)%,\s*transparent\)(\s+transparent)?$/.exec(value);
      if (single) {
        const [, token, percent, tail] = single;
        fallbacks += 1;
        const alpha = `calc(var(${token}-a, 1) * ${Number((Number(percent) / 100).toFixed(3))})`;
        return `${prop}:rgba(var(${token}-rgb), ${alpha})${tail ?? ""};`;
      }
      if (/^color-mix\(in ok(?:lab|lch), currentcolor 50%,\s*transparent\)$/.test(value)) {
        fallbacks += 1;
        return `${prop}:currentColor;`;
      }
      const two = /^color-mix\(in oklch, var\(--[a-z0-9-]+\) [\d.]+%,\s*var\((--[a-z0-9-]+)\)\)$/.exec(value);
      if (two) {
        fallbacks += 1;
        return `${prop}:var(${two[1]});`;
      }
      skipped.push(value);
      return whole;
    },
  );

  css = unwrapColorMixSupports(css);
  const after = { lab: (css.match(/lab\(/g) || []).length, mix: (css.match(/color-mix\(/g) || []).length };
  return { css, before, after, companions, chains, fallbacks, skipped, tokens: [...tokens] };
}

// Tailwind puts every opacity modifier behind `@supports (color: color-mix(...))`. The fallback
// declarations above landed inside those blocks, so a browser without color-mix skipped them and
// kept the solid `bg-primary` base rule. Unwrap the feature test: the contents are plain rgba now.
function unwrapColorMixSupports(css) {
  const marker = /@supports\s*\(\s*color:\s*color-mix\(\s*in lab,\s*red,\s*red\s*\)\s*\)\s*\{/g;
  let current = css;
  for (let pass = 0; pass < 10; pass += 1) {
    let out = "";
    let index = 0;
    marker.lastIndex = 0;
    let match = marker.exec(current);
    while (match) {
      out += current.slice(index, match.index);
      const open = match.index + match[0].length - 1;
      let depth = 1;
      let cursor = open + 1;
      while (cursor < current.length && depth > 0) {
        if (current[cursor] === "{") depth += 1;
        else if (current[cursor] === "}") depth -= 1;
        cursor += 1;
      }
      out += current.slice(open + 1, cursor - 1);
      index = cursor;
      marker.lastIndex = cursor;
      match = marker.exec(current);
    }
    out += current.slice(index);
    if (out === current) break;
    current = out;
  }
  return current;
}

const input = process.argv[2];
const output = process.argv[3];
if (input && output) {
  const result = transform(input);
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, result.css);
  console.log("before", result.before, "after", result.after);
  console.log("companions", result.companions, "chains", result.chains, "fallbacks", result.fallbacks);
  console.log("mix tokens", result.tokens.join(" "));
  console.log("skipped", result.skipped.length, result.skipped.slice(0, 5));
}
module.exports = { transform };
