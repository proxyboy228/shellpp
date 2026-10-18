// Automated contrast test (docs/design-system.md, 7.3): reads tokens.css and
// the built-in terminal themes, composites translucent colors over the
// surface below them and checks every pair from contrast-pairs.ts against
// its threshold with the WCAG 2.x formula, without rounding.
import { describe, expect, it } from 'vitest';
import { composite, contrastRatio, isOpaque, parseColor, type Rgba } from './color';
import {
  TERMINAL_CONTRAST_RULES,
  UI_CONTRAST_RULES,
  type TerminalColorRef,
  type TerminalContrastRule,
  type UiContrastRule,
} from './contrast-pairs';
import { BUILTIN_TERMINAL_THEME_FILES } from './builtin-terminal-themes';
import { defaultKeywordColors, type KeywordCategory } from './keyword-highlight';
import type { AnsiColorName, TerminalTheme } from './terminal-theme';
import { THEME_NAMES, parseThemeTokens, resolveToken, tokensForTheme, type TokenMap } from './tokens';
import tokensCss from './tokens.css?raw';

const tokens = parseThemeTokens(tokensCss);
const terminalThemes = [...BUILTIN_TERMINAL_THEME_FILES];

interface Failure {
  readonly pair: string;
  readonly ratio: number;
}

function format(ratio: number): number {
  // Shown only in failure messages; the comparison uses the exact value.
  return Number(ratio.toFixed(3));
}

/** Contrast of a foreground over a background; translucent layers are composited first. */
function ratioOf(foreground: Rgba, background: Rgba): number {
  if (!isOpaque(background)) {
    throw new Error('Background must be opaque here; composite it over its surface first');
  }
  const top = isOpaque(foreground) ? foreground : composite(foreground, background);
  return contrastRatio(top, background);
}

function checkUiRule(theme: TokenMap, rule: UiContrastRule): { checked: number; failures: Failure[] } {
  const color = (name: string) => parseColor(resolveToken(theme, name));
  const failures: Failure[] = [];
  let checked = 0;
  for (const foreground of rule.foregrounds) {
    for (const background of rule.backgrounds) {
      const layers = rule.under ?? [null];
      for (const surface of layers) {
        const backgroundColor = color(background);
        let effective: Rgba;
        if (surface === null) {
          if (!isOpaque(backgroundColor)) {
            throw new Error(`${background} is translucent; the rule "${rule.name}" must list surfaces under it`);
          }
          effective = backgroundColor;
        } else {
          effective = composite(backgroundColor, color(surface));
        }
        const ratio = ratioOf(color(foreground), effective);
        checked += 1;
        if (!(ratio >= rule.threshold)) {
          const where = surface === null ? background : `${background} over ${surface}`;
          failures.push({ pair: `${foreground} on ${where}`, ratio: format(ratio) });
        }
      }
    }
  }
  return { checked, failures };
}

function terminalColor(theme: TerminalTheme, ref: TerminalColorRef): Rgba {
  switch (ref) {
    case 'selection':
      return composite(parseColor(theme.selection), parseColor(theme.background));
    case 'selectedText':
      return parseColor(theme.selectionForeground ?? theme.foreground);
    case 'background':
    case 'foreground':
    case 'cursor':
    case 'cursorText':
    case 'searchMatch':
    case 'searchMatchActive':
      return parseColor(theme[ref]);
  }
  if (ref.startsWith('ansi.')) {
    return parseColor(theme.ansi[ref.slice('ansi.'.length) as AnsiColorName]);
  }
  return parseColor(defaultKeywordColors(theme.background)[ref.slice('keyword.'.length) as KeywordCategory]);
}

function checkTerminalRule(theme: TerminalTheme, rule: TerminalContrastRule): { checked: number; failures: Failure[] } {
  const failures: Failure[] = [];
  let checked = 0;
  for (const foreground of rule.foregrounds) {
    for (const background of rule.backgrounds) {
      const ratio = ratioOf(terminalColor(theme, foreground), terminalColor(theme, background));
      checked += 1;
      if (!(ratio >= rule.threshold)) {
        failures.push({ pair: `${foreground} on ${background}`, ratio: format(ratio) });
      }
    }
  }
  return { checked, failures };
}

describe('WCAG 2.x contrast formula', () => {
  it('matches reference values', () => {
    const black = parseColor('#000000');
    const white = parseColor('#FFFFFF');
    expect(contrastRatio(black, white)).toBe(21);
    expect(contrastRatio(white, white)).toBe(1);
    expect(contrastRatio(parseColor('#777777'), white)).toBeCloseTo(4.4781, 4);
    expect(contrastRatio(parseColor('#767676'), white)).toBeCloseTo(4.5422, 4);
  });

  it('composites translucent colors over the surface below', () => {
    expect(composite(parseColor('rgba(255, 255, 255, 0.5)'), parseColor('#000000'))).toEqual([127.5, 127.5, 127.5, 1]);
    expect(composite(parseColor('#2EC4A64D'), parseColor('#121211'))[3]).toBe(1);
  });

  it('compares without rounding', () => {
    // White on the old light accent is 4.4994:1, which rounds to 4.5 but fails.
    const ratio = contrastRatio(parseColor('#FFFFFF'), parseColor('#13866F'));
    expect(ratio.toFixed(1)).toBe('4.5');
    expect(ratio >= 4.5).toBe(false);
  });
});

describe.each(THEME_NAMES)('interface contrast, %s theme', (themeName) => {
  const theme = tokensForTheme(tokens, themeName);

  it.each(UI_CONTRAST_RULES.map((rule) => [`${rule.name} (${rule.threshold}:1)`, rule] as const))('%s', (_, rule) => {
    const { checked, failures } = checkUiRule(theme, rule);
    expect(checked).toBeGreaterThan(0);
    expect(failures).toEqual([]);
  });
});

describe('built-in terminal themes', () => {
  it('exist', () => {
    expect(terminalThemes.map(([, theme]) => theme.name).sort()).toEqual(['Graphite', 'Paper']);
  });
});

describe.each(terminalThemes.map(([path, theme]) => [theme.name, path, theme] as const))(
  'terminal contrast, %s (%s)',
  (_name, _path, theme) => {
    it.each(TERMINAL_CONTRAST_RULES.map((rule) => [`${rule.name} (${rule.threshold}:1)`, rule] as const))(
      '%s',
      (_, rule) => {
        const { checked, failures } = checkTerminalRule(theme, rule);
        expect(checked).toBeGreaterThan(0);
        expect(failures).toEqual([]);
      },
    );
  },
);
