// Stylesheets of the terminal feature use only defined tokens and no raw
// colors (docs/design-system.md 7.1: raw color values live in src/theme/;
// terminal colors come from the theme files at run time).

import { describe, expect, it } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';

const sheets = import.meta.glob<string>('./**/*.module.css', { query: '?raw', import: 'default', eager: true });
const sources = import.meta.glob<string>(['./**/*.{ts,tsx}', '!./**/*.test.{ts,tsx}'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));

describe('terminal stylesheets', () => {
  it('are found', () => {
    expect(Object.keys(sheets).length).toBeGreaterThanOrEqual(3);
  });

  it.each(Object.entries(sheets))('%s uses defined tokens and no raw colors', (_path, css) => {
    const used = Array.from(css.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(/);
  });

  it.each(Object.entries(sources))('%s has no raw color literals', (_path, source) => {
    expect(source).not.toMatch(/['"`]#[0-9a-fA-F]{3,8}['"`]|rgba?\(\s*\d|hsla?\(\s*\d/);
  });
});
