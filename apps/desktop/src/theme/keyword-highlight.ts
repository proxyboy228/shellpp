// Default keyword highlighting colors (docs/design-system.md, 4.3).
// The column is chosen by the brightness of the terminal theme's background,
// not by the interface theme. Colors are hex because xterm.js needs them.

import { isLightBackground, parseColor } from './color';

export const KEYWORD_CATEGORIES = ['error', 'warning', 'ok', 'info', 'debug', 'address'] as const;

export type KeywordCategory = (typeof KEYWORD_CATEGORIES)[number];

export type KeywordColors = Readonly<Record<KeywordCategory, string>>;

export const KEYWORD_HIGHLIGHT_DEFAULTS: Readonly<Record<'darkBackground' | 'lightBackground', KeywordColors>> = {
  darkBackground: {
    error: '#F0665E',
    warning: '#E3B04B',
    ok: '#7CC46F',
    info: '#7AA9DB',
    debug: '#A38FD9',
    address: '#D883A6',
  },
  lightBackground: {
    error: '#C0392F',
    warning: '#8D620B',
    ok: '#36782F',
    info: '#2E6AA6',
    debug: '#6B55B0',
    address: '#A84B70',
  },
};

/** The default keyword colors for a terminal background color. */
export function defaultKeywordColors(terminalBackground: string): KeywordColors {
  return isLightBackground(parseColor(terminalBackground))
    ? KEYWORD_HIGHLIGHT_DEFAULTS.lightBackground
    : KEYWORD_HIGHLIGHT_DEFAULTS.darkBackground;
}
