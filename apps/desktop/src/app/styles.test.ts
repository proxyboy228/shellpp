// Stylesheets of the app shell, lock screens and settings use only defined
// tokens and no raw colors (docs/design-system.md 7.1: raw color values are
// allowed only in src/theme/).

import { describe, expect, it } from 'vitest';
import tokensCss from '../theme/tokens.css?raw';

const sheets = {
  ...import.meta.glob<string>('./**/*.module.css', { query: '?raw', import: 'default', eager: true }),
  ...import.meta.glob<string>('../features/lock/**/*.module.css', { query: '?raw', import: 'default', eager: true }),
  ...import.meta.glob<string>('../features/settings/**/*.module.css', { query: '?raw', import: 'default', eager: true }),
  ...import.meta.glob<string>('../components/Menu/*.module.css', { query: '?raw', import: 'default', eager: true }),
};

const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));

describe('W4 stylesheets', () => {
  it('are found', () => {
    expect(Object.keys(sheets).length).toBeGreaterThan(8);
  });

  it.each(Object.entries(sheets))('%s uses defined tokens and no raw colors', (_path, css) => {
    const used = Array.from(css.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(/);
  });
});
