// Structure of tokens.css and the terminal themes, and their agreement with
// docs/design-system.md and docs/ru/design-system.md (section 7.1).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { BUILTIN_TERMINAL_THEME_FILES, BUILTIN_TERMINAL_THEMES, DEFAULT_TERMINAL_THEME_NAME } from './builtin-terminal-themes';
import { formatRgba, isLightBackground, parseColor } from './color';
import { KEYWORD_CATEGORIES, KEYWORD_HIGHLIGHT_DEFAULTS, defaultKeywordColors } from './keyword-highlight';
import { ANSI_COLOR_NAMES, TerminalThemeError, parseTerminalTheme, type TerminalTheme } from './terminal-theme';
import { parseCssRules, parseThemeTokens, resolveToken, tokensForTheme } from './tokens';
import tokensCss from './tokens.css?raw';

const tokens = parseThemeTokens(tokensCss);

function readDoc(relativePath: string): string {
  return readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8');
}

const docs = {
  en: readDoc('../../../../docs/design-system.md'),
  ru: readDoc('../../../../docs/ru/design-system.md'),
};

// --- Helpers for reading the design document -------------------------------

const COLOR_IN_VALUE = /#[0-9a-f]{6}(?:[0-9a-f]{2})?\b|rgba?\([^)]*\)/gi;

/** A canonical form of a CSS value, so that `rgba(0,0,0,0.50)` equals `rgba(0, 0, 0, 0.5)`. */
function canonical(value: string): string {
  return value
    .replace(COLOR_IN_VALUE, (color) => formatRgba(parseColor(color)))
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function canonicalMap(map: ReadonlyMap<string, string>): Map<string, string> {
  return new Map([...map].map(([name, value]) => [name, canonical(value)]));
}

interface TableRow {
  readonly section: string;
  readonly cells: readonly string[];
}

function tableRows(doc: string): TableRow[] {
  const rows: TableRow[] = [];
  let section = '';
  for (const line of doc.split('\n')) {
    const heading = /^#{2,3} (\d+(?:\.\d+)?)\.?\s/.exec(line);
    if (heading !== null) {
      section = heading[1];
    } else if (line.startsWith('|') && !/^\|[\s|:-]+\|$/.test(line)) {
      rows.push({ section, cells: line.split('|').slice(1, -1).map((cell) => cell.trim()) });
    }
  }
  return rows;
}

function codeSpans(text: string): string[] {
  return [...text.matchAll(/`([^`]+)`/g)].map((match) => match[1]);
}

/** Token values from the tables of sections 2 and 3. */
function documentedTokens(doc: string) {
  const shared = new Map<string, string>();
  const dark = new Map<string, string>();
  const light = new Map<string, string>();
  for (const { section, cells } of tableRows(doc)) {
    if (!/^[23]\./.test(section)) continue;
    const [name] = codeSpans(cells[0]);
    if (name === undefined || !name.startsWith('--')) continue;
    if (cells.length === 2) {
      shared.set(name, codeSpans(cells[1])[0]);
    } else {
      dark.set(name, codeSpans(cells[cells.length - 2])[0]);
      light.set(name, codeSpans(cells[cells.length - 1])[0]);
    }
  }
  return { shared, dark, light };
}

/** Values written in prose right before a token, e.g. `12 px (`--font-size-caption`)`. */
function documentedNumbers(doc: string): Map<string, string> {
  const values = new Map<string, string>();
  const text = doc.replace(/\s+/g, ' ');
  // Units: px, and ms (written "мс" in the Russian version).
  for (const [, number, unit, name] of text.matchAll(/(\d+(?:\.\d+)?)(?: (px|ms|мс))? \(`(--[a-z0-9-]+)`/g)) {
    const value = `${number}${unit === 'мс' ? 'ms' : (unit ?? '')}`;
    if (values.has(name) && values.get(name) !== value) {
      throw new Error(`Conflicting values for ${name} in the design document`);
    }
    values.set(name, value);
  }
  return values;
}

/** Role colors of a terminal theme table (sections 4.1 and 4.2); "not set" roles are left out. */
function documentedTerminalTheme(doc: string, section: string): Map<string, string> {
  const roles = new Map<string, string>();
  for (const row of tableRows(doc).filter((r) => r.section === section)) {
    for (let i = 0; i + 1 < row.cells.length; i += 2) {
      const [role] = codeSpans(row.cells[i]);
      const [color] = codeSpans(row.cells[i + 1]);
      if (role !== undefined && color !== undefined) roles.set(role, color);
    }
  }
  return roles;
}

function themeRoles(theme: TerminalTheme): Map<string, string> {
  const roles = new Map<string, string>();
  for (const [key, value] of Object.entries(theme)) {
    if (typeof value === 'string' && key !== 'name') roles.set(key, value);
  }
  for (const [key, value] of Object.entries(theme.ansi)) roles.set(key, value);
  return roles;
}

/** The keyword table of section 4.3, by category in table order. */
function documentedKeywordColors(doc: string) {
  const rows = tableRows(doc)
    .filter((row) => row.section === '4.3')
    .map((row) => row.cells.map((cell) => codeSpans(cell)[0]))
    .filter(([, dark, light]) => dark !== undefined && light !== undefined);
  const columns = (index: number) => Object.fromEntries(KEYWORD_CATEGORIES.map((category, i) => [category, rows[i]?.[index]]));
  return { darkBackground: columns(1), lightBackground: columns(2), rows: rows.length };
}

function jsonExample(doc: string): unknown {
  const match = /```json\n([\s\S]*?)\n```/.exec(doc);
  if (match === null) throw new Error('No JSON example in the design document');
  return JSON.parse(match[1]);
}

// --- tokens.css ------------------------------------------------------------

describe('tokens.css', () => {
  it('defines every token block', () => {
    for (const block of Object.values(tokens)) {
      expect(block.size).toBeGreaterThan(0);
    }
  });

  it('defines the same tokens in the dark and light themes', () => {
    expect([...tokens.light.keys()].sort()).toEqual([...tokens.dark.keys()].sort());
  });

  it('repeats the light theme in the Auto block', () => {
    expect(tokens.autoLight).toEqual(tokens.light);
  });

  it('does not repeat shared tokens in the theme blocks', () => {
    const overlap = [...tokens.dark.keys(), ...tokens.light.keys()].filter((name) => tokens.shared.has(name));
    expect(overlap).toEqual([]);
  });

  it('uses valid color values for every color token', () => {
    for (const theme of [tokens.dark, tokens.light]) {
      for (const [name, value] of theme) {
        if (name.startsWith('--shadow-')) continue;
        expect(() => parseColor(value), name).not.toThrow();
      }
    }
    for (const [name, value] of tokens.shared) {
      if (name.startsWith('--tag-') || name.startsWith('--text-')) {
        expect(() => parseColor(value), name).not.toThrow();
      }
    }
  });

  it('turns off every animation duration under reduced motion', () => {
    const durations = [...tokens.shared.keys()].filter((name) => name.startsWith('--duration-'));
    expect(durations.length).toBeGreaterThan(0);
    expect(new Map([...tokens.reducedMotion].sort())).toEqual(new Map(durations.sort().map((name) => [name, '0ms'])));
  });

  it('has the spacing scale from the design document', () => {
    const spacing = [1, 2, 3, 4, 5, 6].map((step) => tokens.shared.get(`--space-${step}`));
    expect(spacing).toEqual(['4px', '8px', '12px', '16px', '24px', '32px']);
  });
});

// --- Agreement with the design document ------------------------------------

describe('design-system.md', () => {
  const documented = documentedTokens(docs.en);

  it('lists the same color tokens as tokens.css', () => {
    expect(canonicalMap(documented.dark)).toEqual(canonicalMap(tokens.dark));
    expect(canonicalMap(documented.light)).toEqual(canonicalMap(tokens.light));
    const sharedColors = new Map([...tokens.shared].filter(([name]) => documented.shared.has(name)));
    expect(canonicalMap(documented.shared)).toEqual(canonicalMap(sharedColors));
    expect([...documented.shared.keys()].sort()).toEqual(
      [...tokens.shared.keys()].filter((name) => name.startsWith('--tag-') || name === '--text-on-tag').sort(),
    );
  });

  it('mentions every token defined in tokens.css', () => {
    const names = [...tokens.shared.keys(), ...tokens.dark.keys()];
    const missing = names.filter((name) => !docs.en.includes(`\`${name}\``));
    expect(missing).toEqual([]);
  });

  it('gives the same sizes and durations as tokens.css', () => {
    const numbers = documentedNumbers(docs.en);
    expect(numbers.size).toBeGreaterThan(20);
    for (const [name, value] of numbers) {
      expect(tokens.shared.get(name)?.replace(/\s+/g, ''), name).toBe(value);
    }
  });

  it('describes the built-in terminal themes as in their files', () => {
    const bySection = { '4.1': 'Graphite', '4.2': 'Paper' } as const;
    for (const [section, name] of Object.entries(bySection)) {
      const theme = BUILTIN_TERMINAL_THEMES.find((candidate) => candidate.name === name);
      expect(theme, name).toBeDefined();
      expect(canonicalMap(documentedTerminalTheme(docs.en, section))).toEqual(canonicalMap(themeRoles(theme!)));
    }
  });

  it('lists the keyword highlighting defaults', () => {
    const { rows, ...columns } = documentedKeywordColors(docs.en);
    expect(rows).toBe(KEYWORD_CATEGORIES.length);
    expect(columns).toEqual(KEYWORD_HIGHLIGHT_DEFAULTS);
  });

  it('shows the Graphite file as the format example', () => {
    expect(jsonExample(docs.en)).toEqual(BUILTIN_TERMINAL_THEMES.find((theme) => theme.name === 'Graphite'));
  });

  it('has the same values in the Russian version', () => {
    expect(documentedTokens(docs.ru)).toEqual(documented);
    expect(documentedNumbers(docs.ru)).toEqual(documentedNumbers(docs.en));
    for (const section of ['4.1', '4.2']) {
      expect(documentedTerminalTheme(docs.ru, section)).toEqual(documentedTerminalTheme(docs.en, section));
    }
    expect(documentedKeywordColors(docs.ru)).toEqual(documentedKeywordColors(docs.en));
    expect(jsonExample(docs.ru)).toEqual(jsonExample(docs.en));
    const headings = (doc: string) => [...doc.matchAll(/^(#{1,3}) (\d+(?:\.\d+)?)?/gm)].map((m) => `${m[1]} ${m[2] ?? ''}`);
    expect(headings(docs.ru)).toEqual(headings(docs.en));
  });
});

// --- Keyword highlighting ----------------------------------------------------

describe('keyword highlighting defaults', () => {
  it('picks the column by the brightness of the terminal background', () => {
    expect(defaultKeywordColors('#121211')).toBe(KEYWORD_HIGHLIGHT_DEFAULTS.darkBackground);
    expect(defaultKeywordColors('#FAF9F6')).toBe(KEYWORD_HIGHLIGHT_DEFAULTS.lightBackground);
    // The switch point is a relative luminance of about 0.1791.
    expect(isLightBackground(parseColor('#767676'))).toBe(true);
    expect(isLightBackground(parseColor('#757575'))).toBe(false);
  });

  it('matches the semantic colors of the interface theme with the same brightness', () => {
    const pairs = [
      ['error', '--danger'],
      ['warning', '--warning'],
      ['ok', '--success'],
      ['info', '--info'],
    ] as const;
    const dark = tokensForTheme(tokens, 'dark');
    const light = tokensForTheme(tokens, 'light');
    for (const [category, token] of pairs) {
      expect(KEYWORD_HIGHLIGHT_DEFAULTS.darkBackground[category]).toBe(resolveToken(dark, token));
      expect(KEYWORD_HIGHLIGHT_DEFAULTS.lightBackground[category]).toBe(resolveToken(light, token));
    }
  });
});

// --- Terminal theme format ---------------------------------------------------

describe('terminal theme format', () => {
  const graphite = BUILTIN_TERMINAL_THEMES.find((theme) => theme.name === DEFAULT_TERMINAL_THEME_NAME);

  it('loads the built-in themes', () => {
    expect(graphite).toBeDefined();
    for (const [path, theme] of BUILTIN_TERMINAL_THEME_FILES) {
      expect(path).toBe(`./terminal/${theme.name.toLowerCase()}.json`);
      expect(Object.keys(theme.ansi)).toEqual([...ANSI_COLOR_NAMES]);
    }
    const names = BUILTIN_TERMINAL_THEMES.map((theme) => theme.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it('accepts lower-case hex and an optional selection foreground', () => {
    const theme = { ...graphite!, background: '#abcdef', selectionForeground: '#FFFFFF' };
    expect(parseTerminalTheme(theme)).toEqual(theme);
    const withoutSelectionForeground: Record<string, unknown> = { ...theme };
    delete withoutSelectionForeground.selectionForeground;
    expect(parseTerminalTheme(withoutSelectionForeground)).not.toHaveProperty('selectionForeground');
  });

  it.each([
    ['a non-object', [], 'JSON object'],
    ['an unknown field', { extra: true }, 'Unknown field: extra'],
    ['another version', { version: 2 }, 'version'],
    ['an empty name', { name: ' ' }, 'name'],
    ['a long name', { name: 'x'.repeat(65) }, 'name'],
    ['alpha in an opaque color', { background: '#12121180' }, 'background'],
    ['a short hex color', { cursor: '#FFF' }, 'cursor'],
    ['an rgba() color', { selection: 'rgba(0, 0, 0, 0.3)' }, 'selection'],
    ['a missing field', { searchMatch: undefined }, 'searchMatch'],
    ['an invalid selection foreground', { selectionForeground: 'white' }, 'selectionForeground'],
  ])('rejects %s', (_, override, message) => {
    const input = Array.isArray(override) ? override : { ...graphite!, ...override };
    expect(() => parseTerminalTheme(input)).toThrow(TerminalThemeError);
    expect(() => parseTerminalTheme(input)).toThrow(message);
  });

  it('rejects a missing or unknown ANSI color', () => {
    const fifteen: Record<string, string> = { ...graphite!.ansi };
    delete fifteen.brightWhite;
    expect(() => parseTerminalTheme({ ...graphite!, ansi: fifteen })).toThrow('ansi.brightWhite');
    expect(() => parseTerminalTheme({ ...graphite!, ansi: { ...graphite!.ansi, orange: '#FF8800' } })).toThrow(
      'ansi.orange',
    );
  });
});

// --- Parser ------------------------------------------------------------------

describe('tokens parser', () => {
  it('reads rules inside @media and skips comments', () => {
    const rules = parseCssRules(`
      /* comment { with braces } */
      :root, [data-theme="dark"] { --a: #000000; --font: "A; B", serif; }
      @media (prefers-color-scheme: light) { :root:not([data-theme]) { --a: #FFFFFF } }
    `);
    expect(rules).toEqual([
      {
        selector: ':root,[data-theme="dark"]',
        media: null,
        declarations: new Map([
          ['--a', '#000000'],
          ['--font', '"A; B", serif'],
        ]),
      },
      { selector: ':root:not([data-theme])', media: '(prefers-color-scheme: light)', declarations: new Map([['--a', '#FFFFFF']]) },
    ]);
  });

  it('rejects CSS it does not understand', () => {
    expect(() => parseCssRules('.a { .b { color: red } }')).toThrow('Nested rules');
    expect(() => parseCssRules('@supports (display: grid) { .a { color: red } }')).toThrow('Unsupported at-rule');
    expect(() => parseCssRules(':root { --a: 1; --a: 2 }')).toThrow('Duplicate');
    expect(() => parseThemeTokens(':root { --a: 1 }')).toThrow('Expected one');
  });

  it('resolves var() references and reports unknown or circular tokens', () => {
    const map = new Map([
      ['--a', 'var(--b)'],
      ['--b', '#123456'],
      ['--c', 'var(--d)'],
      ['--d', 'var(--c)'],
    ]);
    expect(resolveToken(map, '--a')).toBe('#123456');
    expect(() => resolveToken(map, '--missing')).toThrow('Unknown token');
    expect(() => resolveToken(map, '--c')).toThrow('Circular');
  });
});
