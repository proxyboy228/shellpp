// Loads and validates the built-in terminal themes from ./terminal/*.json.

import { parseTerminalTheme, type TerminalTheme } from './terminal-theme';

const sources = import.meta.glob<unknown>('./terminal/*.json', { eager: true, import: 'default' });

function load(path: string): TerminalTheme {
  try {
    return parseTerminalTheme(sources[path]);
  } catch (error) {
    throw new Error(`Invalid built-in terminal theme ${path}`, { cause: error });
  }
}

/** Built-in themes keyed by their file path (e.g. `./terminal/graphite.json`), sorted by path. */
export const BUILTIN_TERMINAL_THEME_FILES: ReadonlyMap<string, TerminalTheme> = new Map(
  Object.keys(sources)
    .sort()
    .map((path) => [path, load(path)]),
);

export const BUILTIN_TERMINAL_THEMES: readonly TerminalTheme[] = [...BUILTIN_TERMINAL_THEME_FILES.values()];

/** The default theme for new hosts (docs/design-system.md, 4.1). */
export const DEFAULT_TERMINAL_THEME_NAME = 'Graphite';
