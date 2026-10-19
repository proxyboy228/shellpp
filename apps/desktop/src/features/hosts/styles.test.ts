// The stylesheets of the Hosts screen and the panels use design tokens only
// (docs/design-system.md 7.1): no raw colors, and every token exists.

import { describe, expect, it } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import hostsCss from './HostsScreen.module.css?raw';
import panelsCss from '../panels/panels.module.css?raw';
import popoverCss from '../panels/Popover.module.css?raw';

// Custom properties set from terminal theme data on the thumbnail elements.
const LOCAL_PROPERTIES = new Set(['--thumb-bg', '--thumb-fg', '--swatch']);

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1] ?? '').filter(
    (name) => !defined.has(name) && !LOCAL_PROPERTIES.has(name),
  );
}

describe.each([
  ['HostsScreen.module.css', hostsCss],
  ['panels.module.css', panelsCss],
  ['Popover.module.css', popoverCss],
])('%s', (_name, css) => {
  it('has no raw colors', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
  });

  it('uses only defined tokens', () => {
    expect(undefinedTokens(css)).toEqual([]);
  });

  it('turns off motion under prefers-reduced-motion', () => {
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
