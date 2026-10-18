// A small parser for tokens.css: reads the custom properties of each theme
// block so that tests can check values and contrast without a browser.
// It understands the subset of CSS that tokens.css uses: rules with plain
// declarations, comments and `@media` blocks.

export interface CssRule {
  /** Selector with whitespace normalized, e.g. `:root,[data-theme="dark"]`. */
  readonly selector: string;
  /** Media condition with whitespace normalized, or null outside `@media`. */
  readonly media: string | null;
  readonly declarations: ReadonlyMap<string, string>;
}

export type TokenMap = ReadonlyMap<string, string>;

export interface ThemeTokens {
  /** Theme-independent tokens (`:root`). */
  readonly shared: TokenMap;
  /** Dark theme (`:root, [data-theme="dark"]`). */
  readonly dark: TokenMap;
  /** Light theme (`[data-theme="light"]`). */
  readonly light: TokenMap;
  /** Auto with a light system theme (`:root:not([data-theme])` in `prefers-color-scheme: light`). */
  readonly autoLight: TokenMap;
  /** Overrides under `prefers-reduced-motion: reduce`. */
  readonly reducedMotion: TokenMap;
}

export type ThemeName = 'dark' | 'light';

export const THEME_NAMES: readonly ThemeName[] = ['dark', 'light'];

/** The selectors and media conditions of the token blocks, normalized. */
export const TOKEN_BLOCKS = {
  shared: { selector: ':root', media: null },
  dark: { selector: ':root,[data-theme="dark"]', media: null },
  light: { selector: '[data-theme="light"]', media: null },
  autoLight: { selector: ':root:not([data-theme])', media: '(prefers-color-scheme: light)' },
  reducedMotion: { selector: ':root', media: '(prefers-reduced-motion: reduce)' },
} as const satisfies Record<keyof ThemeTokens, { selector: string; media: string | null }>;

function normalizeSelector(selector: string): string {
  return selector.replace(/\s+/g, ' ').replace(/\s*,\s*/g, ',').trim();
}

function normalizeMedia(media: string): string {
  return media.replace(/\s+/g, ' ').replace(/\(\s+/g, '(').replace(/\s+\)/g, ')').trim().toLowerCase();
}

/** Splits on `separator` outside quotes and parentheses. */
function splitTopLevel(text: string, separator: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let start = 0;
  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    if (quote !== null) {
      if (char === quote) quote = null;
    } else if (char === '"' || char === "'") {
      quote = char;
    } else if (char === '(') {
      depth += 1;
    } else if (char === ')') {
      depth -= 1;
    } else if (char === separator && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts;
}

function parseDeclarations(body: string): Map<string, string> {
  const declarations = new Map<string, string>();
  for (const part of splitTopLevel(body, ';')) {
    const text = part.trim();
    if (text === '') continue;
    const colon = text.indexOf(':');
    if (colon <= 0) {
      throw new Error(`Malformed declaration: ${text}`);
    }
    const name = text.slice(0, colon).trim();
    const value = text.slice(colon + 1).replace(/\s+/g, ' ').trim();
    if (declarations.has(name)) {
      throw new Error(`Duplicate declaration ${name}`);
    }
    declarations.set(name, value);
  }
  return declarations;
}

/** Finds the index of the `}` that closes the block opened at `open`. */
function findClosingBrace(css: string, open: number): number {
  let depth = 0;
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === '{') depth += 1;
    else if (css[i] === '}') {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  throw new Error('Unbalanced braces in CSS');
}

function parseBlockContent(css: string, media: string | null, rules: CssRule[]): void {
  let position = 0;
  while (position < css.length) {
    const open = css.indexOf('{', position);
    if (open === -1) {
      if (css.slice(position).trim() !== '') {
        throw new Error(`Unexpected CSS outside a rule: ${css.slice(position).trim()}`);
      }
      return;
    }
    const prelude = css.slice(position, open).trim();
    const close = findClosingBrace(css, open);
    const body = css.slice(open + 1, close);
    if (prelude.startsWith('@media')) {
      if (media !== null) {
        throw new Error('Nested @media blocks are not supported');
      }
      parseBlockContent(body, normalizeMedia(prelude.slice('@media'.length)), rules);
    } else if (prelude.startsWith('@')) {
      throw new Error(`Unsupported at-rule: ${prelude}`);
    } else {
      if (body.includes('{')) {
        throw new Error(`Nested rules are not supported: ${prelude}`);
      }
      rules.push({ selector: normalizeSelector(prelude), media, declarations: parseDeclarations(body) });
    }
    position = close + 1;
  }
}

/** Parses the rules of a stylesheet, including rules inside `@media`. */
export function parseCssRules(css: string): CssRule[] {
  const rules: CssRule[] = [];
  parseBlockContent(css.replace(/\/\*[\s\S]*?\*\//g, ''), null, rules);
  return rules;
}

function customProperties(declarations: ReadonlyMap<string, string>): Map<string, string> {
  return new Map([...declarations].filter(([name]) => name.startsWith('--')));
}

/** Reads the token blocks of tokens.css. Throws if a block is missing or repeated. */
export function parseThemeTokens(css: string): ThemeTokens {
  const rules = parseCssRules(css);
  const pick = (key: keyof ThemeTokens): TokenMap => {
    const { selector, media } = TOKEN_BLOCKS[key];
    const matches = rules.filter((rule) => rule.selector === selector && rule.media === media);
    if (matches.length !== 1) {
      throw new Error(`Expected one "${selector}" block${media === null ? '' : ` in @media ${media}`}, found ${matches.length}`);
    }
    return customProperties(matches[0].declarations);
  };
  return {
    shared: pick('shared'),
    dark: pick('dark'),
    light: pick('light'),
    autoLight: pick('autoLight'),
    reducedMotion: pick('reducedMotion'),
  };
}

/** The tokens in effect for a theme: theme-specific values over the shared ones. */
export function tokensForTheme(tokens: ThemeTokens, theme: ThemeName): TokenMap {
  return new Map([...tokens.shared, ...tokens[theme]]);
}

/** Returns the value of a token, following `var(--name)` references. Throws if it is missing. */
export function resolveToken(tokens: TokenMap, name: string, seen: readonly string[] = []): string {
  if (seen.includes(name)) {
    throw new Error(`Circular token reference: ${[...seen, name].join(' -> ')}`);
  }
  const value = tokens.get(name);
  if (value === undefined) {
    throw new Error(`Unknown token ${name}`);
  }
  const reference = /^var\(\s*(--[\w-]+)\s*\)$/.exec(value);
  return reference === null ? value : resolveToken(tokens, reference[1], [...seen, name]);
}
