#!/usr/bin/env node
// Draws the placeholder application mark as a PNG, with no image library and no
// borrowed artwork. Replace `art/icon.png` with the real mark when it exists and
// re-run `pnpm icons`; nothing else has to change.
//
//   node scripts/generate-icon.mjs   ->   art/icon.png   ->   pnpm icons

import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { deflateSync } from "node:zlib";

import { REPO_ROOT } from "./lib/descriptors.mjs";

const SIZE = 1024;
const OUTPUT = path.join(REPO_ROOT, "art", "icon.png");

const COLOR = {
  plate: [0x12 / 255, 0x16 / 255, 0x1d / 255],
  ring: [0x2b / 255, 0x33 / 255, 0x42 / 255],
  chevron: [0x4d / 255, 0xa3 / 255, 0xff / 255],
  underscore: [0xe6 / 255, 0xeb / 255, 0xf5 / 255],
};

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

function paintRoundedRect(left, top, right, bottom, radius, color) {
  forEachPixelInBox(left, top, right, bottom, (x, y) => {
    const distance = roundedRectDistance(x + 0.5, y + 0.5, left, top, right, bottom, radius);
    const coverage = clamp(0.5 - distance, 0, 1);
    if (coverage > 0) {
      blend(x, y, color, coverage);
    }
  });
}

function paintRing(left, top, right, bottom, radius, lineWidth, color) {
  forEachPixelInBox(left, top, right, bottom, (x, y) => {
    const distance = Math.abs(roundedRectDistance(x + 0.5, y + 0.5, left, top, right, bottom, radius));
    const coverage = clamp(lineWidth / 2 + 0.5 - distance, 0, 1);
    if (coverage > 0) {
      blend(x, y, color, coverage);
    }
  });
}

function paintStroke(points, halfWidth, color) {
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
      blend(x, y, color, coverage);
    }
  });
}

function draw() {
  paintRoundedRect(48, 48, SIZE - 48, SIZE - 48, 208, COLOR.plate);
  paintRing(48, 48, SIZE - 48, SIZE - 48, 208, 4, COLOR.ring);

  // A prompt: a chevron followed by an underscore.
  paintStroke(
    [
      [384, 352],
      [556, 512],
      [384, 672],
    ],
    38,
    COLOR.chevron,
  );
  paintRoundedRect(608, 640, 812, 716, 38, COLOR.underscore);
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

draw();
await mkdir(path.dirname(OUTPUT), { recursive: true });
await writeFile(OUTPUT, encodePng());
console.log(`wrote ${path.relative(REPO_ROOT, OUTPUT).split(path.sep).join("/")}`);
