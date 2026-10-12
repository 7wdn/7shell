#!/usr/bin/env node
// Rasterises `art/icon.svg` into `art/icon.png`, with no image library and no borrowed artwork: it
// reads the drawing out of the SVG and paints it, so the vector file stays the one place the mark
// is defined and this stays a step rather than a second copy of it. `tauri icon` then cuts the
// platform sets out of that PNG.
//
//   node scripts/generate-icon.mjs   ->   art/icon.png   ->   pnpm icons
//
// It understands the shapes that one file contains and nothing else: a single <rect> wearing a
// stroke for the plate, and a single <g> of round-capped, round-joined paths made of M and L. What
// it cannot read is an error, not a guess — a stroke that quietly failed to draw is an icon that
// ships wrong, and nobody looks at an icon twice.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { deflateSync } from "node:zlib";

import { REPO_ROOT } from "./lib/descriptors.mjs";

const ARTWORK = path.join(REPO_ROOT, "art", "icon.svg");
const OUTPUT = path.join(REPO_ROOT, "art", "icon.png");

const drawing = parseDrawing(await readFile(ARTWORK, "utf8"));
const SIZE = drawing.size;

/** Premultiplied RGBA, so compositing is a plain source-over add. */
const canvas = new Float64Array(SIZE * SIZE * 4);

function clamp(value, low, high) {
  return Math.min(high, Math.max(low, value));
}

function blend(x, y, [red, green, blue], alpha) {
  const index = (y * SIZE + x) * 4;
  const inverse = 1 - alpha;
  canvas[index] = red * alpha + canvas[index] * inverse;
  canvas[index + 1] = green * alpha + canvas[index + 1] * inverse;
  canvas[index + 2] = blue * alpha + canvas[index + 2] * inverse;
  canvas[index + 3] = alpha + canvas[index + 3] * inverse;
}

function distanceToSegment(px, py, ax, ay, bx, by) {
  const vx = bx - ax;
  const vy = by - ay;
  const wx = px - ax;
  const wy = py - ay;
  const lengthSquared = vx * vx + vy * vy;
  const t = lengthSquared === 0 ? 0 : clamp((wx * vx + wy * vy) / lengthSquared, 0, 1);
  return Math.hypot(px - (ax + t * vx), py - (ay + t * vy));
}

function roundedRectDistance(px, py, left, top, right, bottom, radius) {
  const halfWidth = (right - left) / 2;
  const halfHeight = (bottom - top) / 2;
  const qx = Math.abs(px - (left + halfWidth)) - (halfWidth - radius);
  const qy = Math.abs(py - (top + halfHeight)) - (halfHeight - radius);
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0));
  const inside = Math.min(Math.max(qx, qy), 0);
  return outside + inside - radius;
}

function forEachPixelInBox(left, top, right, bottom, visit) {
  const fromX = clamp(Math.floor(left) - 1, 0, SIZE - 1);
  const fromY = clamp(Math.floor(top) - 1, 0, SIZE - 1);
  const toX = clamp(Math.ceil(right) + 1, 0, SIZE - 1);
  const toY = clamp(Math.ceil(bottom) + 1, 0, SIZE - 1);
  for (let y = fromY; y <= toY; y += 1) {
    for (let x = fromX; x <= toX; x += 1) {
      visit(x, y);
    }
  }
}

function paintRoundedRect(left, top, right, bottom, radius, colour) {
  forEachPixelInBox(left, top, right, bottom, (x, y) => {
    const distance = roundedRectDistance(x + 0.5, y + 0.5, left, top, right, bottom, radius);
    const coverage = clamp(0.5 - distance, 0, 1);
    if (coverage > 0) {
      blend(x, y, colour, coverage);
    }
  });
}

function paintRing(left, top, right, bottom, radius, lineWidth, colour) {
  forEachPixelInBox(left, top, right, bottom, (x, y) => {
    const distance = Math.abs(roundedRectDistance(x + 0.5, y + 0.5, left, top, right, bottom, radius));
    const coverage = clamp(lineWidth / 2 + 0.5 - distance, 0, 1);
    if (coverage > 0) {
      blend(x, y, colour, coverage);
    }
  });
}

/**
 * Filling everything within halfWidth of the nearest segment is the same set as a stroked polyline
 * with round caps and round joins: the capsules of the segments, plus the disc each one carries at
 * the joint they share. So this one distance test draws all three.
 */
function paintStroke(points, halfWidth, colour) {
  const xs = points.map(([x]) => x);
  const ys = points.map(([, y]) => y);
  const left = Math.min(...xs) - halfWidth;
  const right = Math.max(...xs) + halfWidth;
  const top = Math.min(...ys) - halfWidth;
  const bottom = Math.max(...ys) + halfWidth;

  forEachPixelInBox(left, top, right, bottom, (x, y) => {
    let nearest = Number.POSITIVE_INFINITY;
    for (let index = 0; index < points.length - 1; index += 1) {
      const [ax, ay] = points[index];
      const [bx, by] = points[index + 1];
      nearest = Math.min(nearest, distanceToSegment(x + 0.5, y + 0.5, ax, ay, bx, by));
    }
    const coverage = clamp(halfWidth + 0.5 - nearest, 0, 1);
    if (coverage > 0) {
      blend(x, y, colour, coverage);
    }
  });
}

function draw({ plate, paths, half }) {
  paintRoundedRect(plate.left, plate.top, plate.right, plate.bottom, plate.radius, plate.fill);
  paintRing(plate.left, plate.top, plate.right, plate.bottom, plate.radius, plate.strokeWidth, plate.stroke);
  for (const { points, colour } of paths) {
    paintStroke(points, half, colour);
  }
}

function element(source, name) {
  const match = source.match(new RegExp(`<${name}\\b[^>]*>`));
  if (match === null) {
    throw new Error(`art/icon.svg has no <${name}>`);
  }
  return match[0];
}

function attribute(elementSource, name) {
  const match = elementSource.match(new RegExp(`\\b${name}="([^"]*)"`));
  return match === null ? null : match[1];
}

function number(elementSource, name) {
  const value = attribute(elementSource, name);
  const parsed = value === null ? Number.NaN : Number(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`${name} is missing or not a number: ${elementSource}`);
  }
  return parsed;
}

function colour(elementSource, name) {
  const value = attribute(elementSource, name);
  const match = value === null ? null : /^#([0-9a-f]{6})$/i.exec(value);
  if (match === null) {
    throw new Error(`${name} is missing or not a #rrggbb colour: ${elementSource}`);
  }
  const hex = Number.parseInt(match[1], 16);
  return [((hex >> 16) & 0xff) / 255, ((hex >> 8) & 0xff) / 255, (hex & 0xff) / 255];
}

/** The `d` this file uses: one M, then Ls, each with an absolute x and y. */
function pointList(data) {
  const tokens = data === null ? [] : (data.match(/[A-Za-z]|-?\d*\.?\d+/g) ?? []);
  const points = [];
  let index = 0;
  while (index < tokens.length) {
    const expected = points.length === 0 ? "M" : "L";
    if (tokens[index] !== expected) {
      throw new Error(`expected ${expected} in "${data}", found "${tokens[index]}"`);
    }
    const x = Number(tokens[index + 1]);
    const y = Number(tokens[index + 2]);
    if (!Number.isFinite(x) || !Number.isFinite(y)) {
      throw new Error(`"${expected}" in "${data}" is missing a coordinate`);
    }
    points.push([x, y]);
    index += 3;
  }
  if (points.length < 2) {
    throw new Error(`a path needs at least two points: "${data}"`);
  }
  return points;
}

function parseDrawing(source) {
  const root = element(source, "svg");
  const viewBox = (attribute(root, "viewBox") ?? "").split(/[\s,]+/).map(Number);
  const square = viewBox.length === 4 && viewBox[0] === 0 && viewBox[1] === 0 && viewBox[2] === viewBox[3];
  if (!square) {
    throw new Error(`expected a square viewBox starting at 0 0, got "${attribute(root, "viewBox")}"`);
  }

  const plateSource = element(source, "rect");
  const left = number(plateSource, "x");
  const top = number(plateSource, "y");
  const plate = {
    left,
    top,
    right: left + number(plateSource, "width"),
    bottom: top + number(plateSource, "height"),
    radius: number(plateSource, "rx"),
    fill: colour(plateSource, "fill"),
    stroke: colour(plateSource, "stroke"),
    strokeWidth: number(plateSource, "stroke-width"),
  };

  const group = element(source, "g");
  if (attribute(group, "fill") !== "none") {
    throw new Error(`the <g> has to set fill="none": ${group}`);
  }
  for (const property of ["stroke-linecap", "stroke-linejoin"]) {
    if (attribute(group, property) !== "round") {
      throw new Error(`only ${property}="round" is drawn, not "${attribute(group, property)}"`);
    }
  }

  const paths = [...source.matchAll(/<path\b[^>]*>/g)].map(([pathSource]) => ({
    points: pointList(attribute(pathSource, "d")),
    colour: colour(pathSource, "stroke"),
  }));
  if (paths.length === 0) {
    throw new Error("art/icon.svg draws no <path>");
  }

  return { size: viewBox[2], plate, paths, half: number(group, "stroke-width") / 2 };
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([length, body, checksum]);
}

function encodePng() {
  const stride = SIZE * 4;
  const raw = Buffer.alloc((stride + 1) * SIZE);
  for (let y = 0; y < SIZE; y += 1) {
    const rowStart = y * (stride + 1);
    raw[rowStart] = 0;
    for (let x = 0; x < SIZE; x += 1) {
      const index = (y * SIZE + x) * 4;
      const alpha = canvas[index + 3];
      const offset = rowStart + 1 + x * 4;
      for (let channel = 0; channel < 3; channel += 1) {
        const value = alpha > 0 ? canvas[index + channel] / alpha : 0;
        raw[offset + channel] = Math.round(clamp(value, 0, 1) * 255);
      }
      raw[offset + 3] = Math.round(clamp(alpha, 0, 1) * 255);
    }
  }

  const header = Buffer.alloc(13);
  header.writeUInt32BE(SIZE, 0);
  header.writeUInt32BE(SIZE, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // colour type: RGBA
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

draw(drawing);
await mkdir(path.dirname(OUTPUT), { recursive: true });
await writeFile(OUTPUT, encodePng());
console.log(`wrote ${path.relative(REPO_ROOT, OUTPUT).split(path.sep).join("/")} at ${SIZE}px from art/icon.svg`);
