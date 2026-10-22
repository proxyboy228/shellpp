#!/usr/bin/env node
// Renders the Shell++ app icon (D83, item 3: variant B, an accent tile with a
// terminal window, its prompt and a plus) into the platform icon files of
// apps/desktop/assets/icons/:
//
// - icon.ico: 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256 px, PNG entries
//   (Windows Vista and later); embedded in shellpp.exe as icon 1, which is
//   also the window's icon (apps/desktop/build.rs);
// - icon.icns: 16 to 1024 px for macOS;
// - icon.png (512 px), 32x32.png, 128x128.png and 128x128@2x.png.
//
// Two masters: source.svg (docs/assets/icon-b-window.svg) from 32 px up, and
// source-small.svg for 16 to 24 px, the same tile and window drawn on the
// 16 px grid without the title bar's dots and the plus, which do not survive
// at that size.
//
// The SVGs are rasterized by the resvg command line tool
// (`cargo install resvg --version 0.46.0 --locked`).
//
// Usage: node scripts/gen-icon.mjs [resvg]

import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');
const icons = join(repo, 'apps', 'desktop', 'assets', 'icons');
const resvg = process.argv[2] ?? 'resvg';

/** The largest size drawn from the small master. */
const SMALL_UP_TO = 24;
const ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
// OSType and pixel size of each icns entry (PNG data).
const ICNS_ENTRIES = [
  ['icp4', 16],
  ['icp5', 32],
  ['ic11', 32],
  ['ic12', 64],
  ['ic07', 128],
  ['ic13', 256],
  ['ic08', 256],
  ['ic14', 512],
  ['ic09', 512],
  ['ic10', 1024],
];

const scratch = mkdtempSync(join(tmpdir(), 'shellpp-icon-'));
const rendered = new Map();

/** The PNG of the icon at `size` px. */
function png(size) {
  if (!rendered.has(size)) {
    const master = size <= SMALL_UP_TO ? 'source-small.svg' : 'source.svg';
    const out = join(scratch, `${size}.png`);
    execFileSync(resvg, ['-w', `${size}`, '-h', `${size}`, join(icons, master), out]);
    rendered.set(size, readFileSync(out));
  }
  return rendered.get(size);
}

/** An .ico of PNG entries. */
function ico(sizes) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  const entries = [];
  const images = [];
  let offset = 6 + 16 * sizes.length;
  for (const size of sizes) {
    const data = png(size);
    const entry = Buffer.alloc(16);
    entry.writeUInt8(size >= 256 ? 0 : size, 0);
    entry.writeUInt8(size >= 256 ? 0 : size, 1);
    entry.writeUInt8(0, 2);
    entry.writeUInt8(0, 3);
    entry.writeUInt16LE(1, 4);
    entry.writeUInt16LE(32, 6);
    entry.writeUInt32LE(data.length, 8);
    entry.writeUInt32LE(offset, 12);
    offset += data.length;
    entries.push(entry);
    images.push(data);
  }
  return Buffer.concat([header, ...entries, ...images]);
}

/** An .icns of PNG entries. */
function icns(entries) {
  const chunks = entries.map(([type, size]) => {
    const data = png(size);
    const head = Buffer.alloc(8);
    head.write(type, 0, 'ascii');
    head.writeUInt32BE(data.length + 8, 4);
    return Buffer.concat([head, data]);
  });
  const body = Buffer.concat(chunks);
  const head = Buffer.alloc(8);
  head.write('icns', 0, 'ascii');
  head.writeUInt32BE(body.length + 8, 4);
  return Buffer.concat([head, body]);
}

try {
  writeFileSync(join(icons, 'icon.ico'), ico(ICO_SIZES));
  writeFileSync(join(icons, 'icon.icns'), icns(ICNS_ENTRIES));
  writeFileSync(join(icons, 'icon.png'), png(512));
  writeFileSync(join(icons, '32x32.png'), png(32));
  writeFileSync(join(icons, '128x128.png'), png(128));
  writeFileSync(join(icons, '128x128@2x.png'), png(256));
  console.log(`gen-icon: wrote the icon files to ${icons}`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
