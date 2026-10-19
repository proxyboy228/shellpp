#!/usr/bin/env node
// Renders the Shell++ placeholder app icon (apps/desktop/assets/icons/source.svg)
// to a square RGBA PNG without any image library: the shapes are evaluated as
// signed distance fields with analytic anti-aliasing and the PNG is written
// with node:zlib.
//
// The icon is original artwork: a graphite rounded square with a jade ">"
// chevron and an underscore. Keep the geometry in sync with source.svg.
//
// Usage: node scripts/gen-icon.mjs <out.png> [size=1024]
// The platform icon files in apps/desktop/assets/icons/ (.ico, .icns, sized
// PNGs) are derived from that PNG with any icon converter.

import { writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const VIEWBOX = 1024;
const GRAPHITE = [0x12, 0x12, 0x11];
const JADE = [0x2e, 0xc4, 0xa6];

// Geometry in viewBox units (matches source.svg).
const TILE = { x: 64, y: 64, size: 896, radius: 192 };
const STROKE_WIDTH = 88;
const CHEVRON = [
  [304, 352],
  [496, 512],
  [304, 672],
];
const UNDERSCORE = [
  [560, 672],
  [736, 672],
];

function roundedSquareDistance(px, py) {
  const half = TILE.size / 2;
  const cx = TILE.x + half;
  const cy = TILE.y + half;
  const qx = Math.abs(px - cx) - (half - TILE.radius);
  const qy = Math.abs(py - cy) - (half - TILE.radius);
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0));
  const inside = Math.min(Math.max(qx, qy), 0);
  return outside + inside - TILE.radius;
}

function segmentDistance(px, py, [ax, ay], [bx, by]) {
  const abx = bx - ax;
  const aby = by - ay;
  const t = Math.max(0, Math.min(1, ((px - ax) * abx + (py - ay) * aby) / (abx * abx + aby * aby)));
  return Math.hypot(px - (ax + t * abx), py - (ay + t * aby));
}

function strokeDistance(px, py) {
  let distance = Infinity;
  for (let i = 0; i + 1 < CHEVRON.length; i += 1) {
    distance = Math.min(distance, segmentDistance(px, py, CHEVRON[i], CHEVRON[i + 1]));
  }
  distance = Math.min(distance, segmentDistance(px, py, UNDERSCORE[0], UNDERSCORE[1]));
  return distance - STROKE_WIDTH / 2;
}

/** Coverage of a shape at a pixel from its signed distance in output pixels. */
function coverage(distancePx) {
  return Math.max(0, Math.min(1, 0.5 - distancePx));
}

function render(size) {
  const scale = size / VIEWBOX;
  const rgba = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const px = (x + 0.5) / scale;
      const py = (y + 0.5) / scale;
      const tile = coverage(roundedSquareDistance(px, py) * scale);
      const stroke = coverage(strokeDistance(px, py) * scale);
      // The stroke is drawn over the tile ("source over"); both are opaque colors.
      const alpha = stroke + tile * (1 - stroke);
      const offset = (y * size + x) * 4;
      for (let channel = 0; channel < 3; channel += 1) {
        const premultiplied = JADE[channel] * stroke + GRAPHITE[channel] * tile * (1 - stroke);
        rgba[offset + channel] = alpha > 0 ? Math.round(premultiplied / alpha) : 0;
      }
      rgba[offset + 3] = Math.round(alpha * 255);
    }
  }
  return rgba;
}

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function encodePng(size, rgba) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // color type: RGBA
  header[10] = 0; // compression
  header[11] = 0; // filter
  header[12] = 0; // no interlace
  const stride = size * 4;
  const raw = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y += 1) {
    raw[y * (stride + 1)] = 0; // filter type: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const [outPath, sizeArgument] = process.argv.slice(2);
if (outPath === undefined) {
  console.error('Usage: node scripts/gen-icon.mjs <out.png> [size=1024]');
  process.exit(2);
}
const size = Number.parseInt(sizeArgument ?? '1024', 10);
if (!Number.isInteger(size) || size < 16 || size > 4096) {
  console.error('size must be an integer from 16 to 4096');
  process.exit(2);
}
writeFileSync(outPath, encodePng(size, render(size)));
console.log(`Wrote ${outPath} (${size}x${size})`);
