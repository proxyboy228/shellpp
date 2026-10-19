#!/usr/bin/env node
// Prints the features hidden in this build: the entries of FEATURE_STAGES in
// apps/desktop/src/stage.ts whose stage is greater than BUILD_STAGE
// (docs/architecture.md, section 6.2). The stage report includes the output.
//
//   node scripts/hidden-features.mjs [--json]
//
// The parser expects one `'feature-id': <stage | LATER>,` entry per line, as
// stage.ts documents; `// UI ...` comment lines name the UI area of the
// entries below them.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const STAGE_FILE = path.join(REPO_ROOT, 'apps', 'desktop', 'src', 'stage.ts');

/** Parses stage.ts: `{buildStage, later, features: [{feature, stage, later, area}]}`. */
export function parseStageFile(source = fs.readFileSync(STAGE_FILE, 'utf8')) {
  const buildStage = Number(/export const BUILD_STAGE\s*=\s*(\d+)/.exec(source)?.[1]);
  const later = Number(/export const LATER\s*=\s*(\d+)/.exec(source)?.[1]);
  if (!Number.isFinite(buildStage) || !Number.isFinite(later)) {
    throw new Error(`BUILD_STAGE or LATER not found in ${STAGE_FILE}`);
  }
  const block = /FEATURE_STAGES\s*=\s*\{([\s\S]*?)\}\s*as const/.exec(source)?.[1];
  if (block === undefined) {
    throw new Error(`FEATURE_STAGES not found in ${STAGE_FILE}`);
  }
  const features = [];
  let area = '';
  for (const rawLine of block.split(/\r?\n/)) {
    const line = rawLine.trim();
    const comment = /^\/\/\s*(.*)$/.exec(line);
    if (comment) {
      area = comment[1];
      continue;
    }
    const entry = /^'([a-z0-9-]+)'\s*:\s*(\d+|LATER)\s*,?/.exec(line);
    if (entry) {
      const isLater = entry[2] === 'LATER';
      features.push({ feature: entry[1], stage: isLater ? later : Number(entry[2]), later: isLater, area });
    } else if (line !== '') {
      throw new Error(`Unexpected line in FEATURE_STAGES: ${line}`);
    }
  }
  return { buildStage, later, features };
}

/** Features with stage > BUILD_STAGE, sorted by stage then id. */
export function hiddenFeatures() {
  const { buildStage, features } = parseStageFile();
  return {
    buildStage,
    hidden: features
      .filter((entry) => entry.stage > buildStage)
      .sort((a, b) => a.stage - b.stage || a.feature.localeCompare(b.feature)),
  };
}

function main(argv) {
  const { buildStage, hidden } = hiddenFeatures();
  if (argv.includes('--json')) {
    console.log(JSON.stringify({ buildStage, hidden }, null, 2));
    return;
  }
  console.log(`Hidden in stage ${buildStage} (${hidden.length} features):`);
  console.log('');
  console.log('| Feature | Stage | Area |');
  console.log('|---|---|---|');
  for (const entry of hidden) {
    console.log(`| \`${entry.feature}\` | ${entry.later ? 'LATER' : entry.stage} | ${entry.area} |`);
  }
}

const invokedDirectly = process.argv[1] !== undefined && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedDirectly) {
  main(process.argv.slice(2));
}
