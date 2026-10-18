// The terminal theme format (docs/design-system.md, 7.2) and its validation.
// Built-in themes live in ./terminal/*.json; imported themes are converted
// to this format and validated with `parseTerminalTheme` before use.

/** The current format version. */
export const TERMINAL_THEME_VERSION = 1;

/** The 16 ANSI colors in color index order (0–15). */
export const ANSI_COLOR_NAMES = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const;

export type AnsiColorName = (typeof ANSI_COLOR_NAMES)[number];

export interface TerminalTheme {
  readonly name: string;
  readonly version: typeof TERMINAL_THEME_VERSION;
  readonly background: string;
  readonly foreground: string;
  readonly cursor: string;
  readonly cursorText: string;
  /** May carry alpha (`#RRGGBBAA`); drawn over `background`. */
  readonly selection: string;
  /** Text color inside the selection. When absent, text keeps its colors. */
  readonly selectionForeground?: string;
  readonly searchMatch: string;
  readonly searchMatchActive: string;
  readonly ansi: Readonly<Record<AnsiColorName, string>>;
}

/** Fields with an opaque `#RRGGBB` color. */
const OPAQUE_COLOR_FIELDS = ['background', 'foreground', 'cursor', 'cursorText', 'searchMatch', 'searchMatchActive'] as const;

const TOP_LEVEL_FIELDS: ReadonlySet<string> = new Set([
  'name',
  'version',
  ...OPAQUE_COLOR_FIELDS,
  'selection',
  'selectionForeground',
  'ansi',
]);

const OPAQUE_HEX_RE = /^#[0-9a-f]{6}$/i;
const HEX_WITH_ALPHA_RE = /^#[0-9a-f]{6}([0-9a-f]{2})?$/i;
const MAX_NAME_LENGTH = 64;

export class TerminalThemeError extends Error {
  override name = 'TerminalThemeError';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function readColor(source: Record<string, unknown>, field: string, path: string, pattern: RegExp, format: string): string {
  const value = source[field];
  if (typeof value !== 'string' || !pattern.test(value)) {
    throw new TerminalThemeError(`${path}${field}: expected a ${format} color`);
  }
  return value;
}

/**
 * Validates a parsed JSON value against the terminal theme format, version 1.
 * Unknown fields and unknown versions are rejected.
 */
export function parseTerminalTheme(input: unknown): TerminalTheme {
  if (!isRecord(input)) {
    throw new TerminalThemeError('A terminal theme must be a JSON object');
  }
  for (const field of Object.keys(input)) {
    if (!TOP_LEVEL_FIELDS.has(field)) {
      throw new TerminalThemeError(`Unknown field: ${field}`);
    }
  }

  if (input.version !== TERMINAL_THEME_VERSION) {
    throw new TerminalThemeError(`version: expected ${TERMINAL_THEME_VERSION}`);
  }
  const { name } = input;
  if (typeof name !== 'string' || name.trim() === '' || name.length > MAX_NAME_LENGTH) {
    throw new TerminalThemeError(`name: expected a non-empty string of at most ${MAX_NAME_LENGTH} characters`);
  }

  const colors = Object.fromEntries(
    OPAQUE_COLOR_FIELDS.map((field) => [field, readColor(input, field, '', OPAQUE_HEX_RE, '#RRGGBB')]),
  ) as Record<(typeof OPAQUE_COLOR_FIELDS)[number], string>;
  const selection = readColor(input, 'selection', '', HEX_WITH_ALPHA_RE, '#RRGGBB or #RRGGBBAA');
  const selectionForeground =
    input.selectionForeground === undefined
      ? undefined
      : readColor(input, 'selectionForeground', '', OPAQUE_HEX_RE, '#RRGGBB');

  const { ansi } = input;
  if (!isRecord(ansi)) {
    throw new TerminalThemeError('ansi: expected an object with the 16 ANSI colors');
  }
  const allowed: ReadonlySet<string> = new Set(ANSI_COLOR_NAMES);
  for (const field of Object.keys(ansi)) {
    if (!allowed.has(field)) {
      throw new TerminalThemeError(`Unknown field: ansi.${field}`);
    }
  }
  const ansiColors = Object.fromEntries(
    ANSI_COLOR_NAMES.map((field) => [field, readColor(ansi, field, 'ansi.', OPAQUE_HEX_RE, '#RRGGBB')]),
  ) as Record<AnsiColorName, string>;

  return {
    name,
    version: TERMINAL_THEME_VERSION,
    ...colors,
    selection,
    ...(selectionForeground === undefined ? {} : { selectionForeground }),
    ansi: ansiColors,
  };
}
