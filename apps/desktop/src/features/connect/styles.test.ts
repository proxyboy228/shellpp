// The stylesheets of the connection screen, the new tab page and Known Hosts
// use design tokens only (docs/design-system.md 7.1): no raw colors, every
// token exists, and motion is off under prefers-reduced-motion.

import { describe, expect, it } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import connectCss from './connect.module.css?raw';
import knownHostsCss from '../knownhosts/knownHosts.module.css?raw';
import newtabCss from '../newtab/newtab.module.css?raw';

const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));

describe.each([
  ['connect.module.css', connectCss],
  ['knownHosts.module.css', knownHostsCss],
  ['newtab.module.css', newtabCss],
])('%s', (_name, css) => {
  it('has no raw colors', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
  });

  it('uses only defined tokens', () => {
    const used = Array.from(css.matchAll(/var\((--[\w-]+)/g), (match) => match[1] ?? '');
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
  });

  it('turns off motion under prefers-reduced-motion', () => {
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
