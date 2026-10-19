#!/usr/bin/env node
// Lists the UI behavior spec sections of the current stage that have no e2e
// spec file (docs/architecture.md, section 9.4; stage 1 and stage 2 plans,
// section 8).
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
// - Stage scope items without a UI section of their own (SCOPE_ITEMS, for
//   example the local terminal of stage 2) must be covered by a spec whose
//   file name or text matches the item.
// - RUST_ONLY lists the stage items that only Rust tests cover (no UI or no
//   test environment for them); the stage report includes the list.
// Exit code 1 when a section or scope item of the current stage has no spec
// (unless --warn-only).

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

/**
 * Stage scope items (docs/architecture.md 6.1) that have no UI section of
 * their own. `file` matches spec file names, `text` matches spec contents.
 */
const SCOPE_ITEMS = [
  { id: 'local-terminal', title: 'Local terminal', stage: 2, file: /local-terminal/ },
  { id: 'os-detection', title: 'OS detection (banner and exec script, UI 12.4 block)', stage: 2, text: /bannerComment|exec:\s*\{\s*stdout/ },
  { id: 'host-key-tofu', title: 'Known hosts and TOFU', stage: 2, file: /^10\.2-/ },
  { id: 'autoreconnect', title: 'Autoreconnect (UI 12.4 default, no settings screen)', stage: 2, text: /session-dropped-countdown/ },
  { id: 'quick-connect', title: 'Quick connect (UI 2.1)', stage: 2, text: /hosts-search-connect/ },
  { id: 'key-path', title: 'Short "+ -> Key" path (UI 3.1)', stage: 2, text: /host-panel-credentials-menu-key|linkPastedKey/ },
];

/** Stage items covered by Rust tests only (stage 2 plan, section 8.2). */
const RUST_ONLY = [
  { stage: 2, item: 'System ssh-agent / Windows OpenSSH agent / Pageant and agent forwarding', tests: 'crates/ssh/tests (agent), tests/sshd agent-keys' },
  { stage: 2, item: 'Keepalive interval and keepalive timeouts', tests: 'crates/ssh/tests' },
  { stage: 2, item: 'No SHA-1 key exchange or signature algorithms', tests: 'crates/ssh/tests' },
  { stage: 2, item: 'Charset conversion (host charset) of output and input', tests: 'apps/desktop/src-tauri/tests' },
  { stage: 2, item: 'Every history secret detector rule (security.md 10.2, S1-S11)', tests: 'crates/core/tests/history.rs' },
  { stage: 2, item: 'Wildcard and IPv6 known host matching', tests: 'crates/core/tests/known_hosts_store.rs, crates/core/src/known_hosts' },
  { stage: 2, item: 'Reconnect attempts waiting for unlock', tests: 'apps/desktop/src-tauri/tests' },
];

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

function specText(name) {
  try {
    return fs.readFileSync(path.join(SPECS_DIR, name), 'utf8');
  } catch {
    return '';
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
  const texts = new Map(specs.map((name) => [name, specText(name)]));
  const scope = SCOPE_ITEMS.map((item) => ({
    id: item.id,
    title: item.title,
    stage: item.stage,
    required: item.stage <= stage,
    specs: specs.filter(
      (name) => (item.file !== undefined && item.file.test(name)) || (item.text !== undefined && item.text.test(texts.get(name) ?? '')),
    ),
  }));
  const rustOnly = RUST_ONLY.filter((entry) => entry.stage <= stage);
  return { buildStage: stage, rows, unmatched, scope, rustOnly };
}

export function main(argv = process.argv.slice(2)) {
  const args = new Set(argv);
  const result = coverage();
  const missing = result.rows.filter((row) => row.required && row.specs.length === 0);
  const missingScope = result.scope.filter((item) => item.required && item.specs.length === 0);
  if (args.has('--json')) {
    console.log(
      JSON.stringify(
        {
          buildStage: result.buildStage,
          missing,
          missingScope,
          rows: result.rows,
          scope: result.scope,
          rustOnly: result.rustOnly,
          unmatched: result.unmatched,
        },
        null,
        2,
      ),
    );
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
    const scopeShown = args.has('--all') ? result.scope : result.scope.filter((item) => item.required);
    if (scopeShown.length > 0) {
      console.log('');
      console.log('Stage scope items without a UI section of their own:');
      console.log('');
      console.log('| Item | Stage | Specs |');
      console.log('|---|---|---|');
      for (const item of scopeShown) {
        const spec = item.specs.length > 0 ? item.specs.join(', ') : item.required ? 'MISSING' : '-';
        console.log(`| ${item.title} | ${item.stage} | ${spec} |`);
      }
    }
    if (result.rustOnly.length > 0) {
      console.log('');
      console.log('Covered by Rust tests only:');
      console.log('');
      console.log('| Item | Stage | Tests |');
      console.log('|---|---|---|');
      for (const entry of result.rustOnly) {
        console.log(`| ${entry.item} | ${entry.stage} | ${entry.tests} |`);
      }
    }
    console.log('');
    if (missing.length === 0) {
      console.log(`Every UI section of stage <= ${result.buildStage} has a spec.`);
    } else {
      console.log(`Sections without a spec: ${missing.map((row) => row.section).join(', ')}`);
    }
    if (missingScope.length > 0) {
      console.log(`Scope items without a spec: ${missingScope.map((item) => item.id).join(', ')}`);
    }
    if (result.unmatched.length > 0) {
      console.log(`Spec files that match no UI section: ${result.unmatched.join(', ')}`);
    }
    const unknown = result.rows.filter((row) => row.stage === null);
    if (unknown.length > 0) {
      console.log(`Sections without a stage in SECTION_STAGES: ${unknown.map((row) => row.section).join(', ')}`);
    }
  }
  if ((missing.length > 0 || missingScope.length > 0) && !args.has('--warn-only')) {
    process.exitCode = 1;
  }
}

const invokedDirectly = process.argv[1] !== undefined && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;
if (invokedDirectly) {
  main();
}
