#!/usr/bin/env node
// Lists the UI sections of the current stage without an e2e spec
// (docs/architecture.md, section 9.4). Same as scripts/e2e-coverage.mjs,
// under the name the stage tasks use.
//
//   node scripts/e2e-missing-specs.mjs [--json] [--all] [--warn-only]

import { main } from './e2e-coverage.mjs';

main(process.argv.slice(2));
