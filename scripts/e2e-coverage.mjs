#!/usr/bin/env node
// Lists the UI behavior spec sections of the current stage that have no e2e
// spec file (docs/architecture.md, section 9.4; stage plan, section 8.1).
//
//   node scripts/e2e-coverage.mjs [--json] [--all] [--warn-only]
//
// - Sections are the `### n.m` headings of docs/ui-behavior-spec.md (and a
//   `## n.` heading without subsections, such as "6. New tab page").
// - A section belongs to the stage in SECTION_STAGES (from SPEC "Work order"
//   and docs/architecture.md 6.1); only sections with stage <= BUILD_STAGE
//   (apps/desktop/src/stage.ts) must have a spec. --all lists every section.
// - A spec covers a section when its file name starts with "<n.m>-" in
//   apps/desktop/e2e/specs (for example `13.1-first-run.e2e.ts`).
// Exit code 1 when a section of the current stage has no spec (unless
// --warn-only).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const UI_SPEC = path.join(REPO_ROOT, 'docs', 'ui-behavior-spec.md');
const STAGE_FILE = path.join(REPO_ROOT, 'apps', 'desktop', 'src', 'stage.ts');
const SPECS_DIR = path.join(REPO_ROOT, 'apps', 'desktop', 'e2e', 'specs');

/**
 * Stage in which each UI section is implemented. A key without a dot covers
 * every subsection of that chapter unless a more specific key exists.
 */
const SECTION_STAGES = {
  '0': 1, // platform notes, checked by the specs of each section
  '1': 1, // main window: 1.1-1.5
  '2': 1, // hosts screen: 2.1-2.4
  '2.5': 3, // import dialog
  '2.6': 8, // move or copy to a vault
  '3': 1, // host and group panels
  '4': 2, // terminal session
  '4.5': 9, // workspaces
  '5': 4, // SFTP
  '6': 2, // new tab page
  '7': 3, // keychain
  '8': 5, // port forwarding
  '9': 5, // snippets
  '9.5': 10, // script generation (later stage)
  '10': 2, // known hosts
  '11': 6, // logs
  '12': 6, // settings sections
  '12.1': 1, // settings window frame (Security only in stage 1)
  '12.2': 3, // account & sync (data export first; sync in stage 7)
  '12.3': 8, // SSH ID and Team
  '12.7': 9, // updates
  '13': 1, // lock and account: 13.1-13.4
  '13.5': 7, // connect sync server wizard
  '13.6': 7, // sync conflicts
};

function buildStage() {
  const source = fs.readFileSync(STAGE_FILE, 'utf8');
  const match = /export const BUILD_STAGE\s*=\s*(\d+)/.exec(source);
  if (!match) {
    throw new Error(`BUILD_STAGE not found in ${STAGE_FILE}`);
  }
  return Number(match[1]);
}

function stageOf(section) {
  if (Object.hasOwn(SECTION_STAGES, section)) {
    return SECTION_STAGES[section];
  }
  const chapter = section.split('.')[0];
  if (Object.hasOwn(SECTION_STAGES, chapter)) {
    return SECTION_STAGES[chapter];
  }
  return undefined;
}

/** `[{section: '1.1', title: 'Top bar (window title bar)'}]` in document order. */
export function uiSections() {
  const lines = fs.readFileSync(UI_SPEC, 'utf8').split(/\r?\n/);
  const chapters = [];
  for (const line of lines) {
    const chapter = /^## (\d+)\.\s+(.*)$/.exec(line);
    if (chapter) {
      chapters.push({ section: chapter[1], title: chapter[2].trim(), subsections: [] });
      continue;
    }
    const sub = /^### (\d+\.\d+)\s+(.*)$/.exec(line);
    if (sub && chapters.length > 0) {
      chapters.at(-1).subsections.push({ section: sub[1], title: sub[2].replace(/\s*\[[^\]]*\]\s*$/, '').trim() });
    }
  }
  const sections = [];
  for (const chapter of chapters) {
    if (chapter.section === '0') {
      continue; // platform notes, not a screen
    }
    if (chapter.subsections.length === 0) {
      sections.push({ section: chapter.section, title: chapter.title.replace(/\s*\[[^\]]*\]\s*$/, '').trim() });
    } else {
      sections.push(...chapter.subsections);
    }
  }
  return sections;
}

/** Spec files in apps/desktop/e2e/specs. */
export function specFiles() {
  try {
    return fs.readdirSync(SPECS_DIR).filter((name) => name.endsWith('.e2e.ts')).sort();
  } catch {
    return [];
  }
}

export function coverage() {
  const stage = buildStage();
  const specs = specFiles();
  const rows = uiSections().map(({ section, title }) => {
    const sectionStage = stageOf(section);
    const files = specs.filter((name) => name.startsWith(`${section}-`));
    return { section, title, stage: sectionStage ?? null, required: sectionStage !== undefined && sectionStage <= stage, specs: files };
  });
  const known = new Set(rows.map((row) => row.section));
  const unmatched = specs.filter((name) => !known.has(name.split('-')[0]));
  return { buildStage: stage, rows, unmatched };
}

export function main(argv = process.argv.slice(2)) {
  const args = new Set(argv);
  const result = coverage();
  const missing = result.rows.filter((row) => row.required && row.specs.length === 0);
  if (args.has('--json')) {
    console.log(JSON.stringify({ buildStage: result.buildStage, missing, rows: result.rows, unmatched: result.unmatched }, null, 2));
  } else {
    const shown = args.has('--all') ? result.rows : result.rows.filter((row) => row.required);
    console.log(`UI sections for stage <= ${result.buildStage}${args.has('--all') ? ' (all sections listed)' : ''}:`);
    console.log('');
    console.log('| Section | Title | Stage | Spec |');
    console.log('|---|---|---|---|');
    for (const row of shown) {
      const spec = row.specs.length > 0 ? row.specs.join(', ') : row.required ? 'MISSING' : '-';
      console.log(`| ${row.section} | ${row.title} | ${row.stage ?? '?'} | ${spec} |`);
    }
    console.log('');
    if (missing.length === 0) {
      console.log(`Every UI section of stage <= ${result.buildStage} has a spec.`);
    } else {
      console.log(`Sections without a spec: ${missing.map((row) => row.section).join(', ')}`);
    }
    if (result.unmatched.length > 0) {
      console.log(`Spec files that match no UI section: ${result.unmatched.join(', ')}`);
    }
    const unknown = result.rows.filter((row) => row.stage === null);
    if (unknown.length > 0) {
      console.log(`Sections without a stage in SECTION_STAGES: ${unknown.map((row) => row.section).join(', ')}`);
    }
  }
  if (missing.length > 0 && !args.has('--warn-only')) {
    process.exitCode = 1;
  }
}

const invokedDirectly = process.argv[1] !== undefined && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedDirectly) {
  main();
}
