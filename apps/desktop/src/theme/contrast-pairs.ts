// The contrast pairs checked by contrast.test.ts. This list mirrors
// docs/design-system.md, 7.3; change both together.
//
// Thresholds are WCAG 2.x contrast ratios, compared without rounding.
// Translucent backgrounds are composited over each surface in `under`
// before the ratio is computed.

import type { AnsiColorName } from './terminal-theme';
import type { KeywordCategory } from './keyword-highlight';

/** Regular text and small text: 4.5:1 (WCAG AA). */
export const TEXT = 4.5;
/** Icons, indicators, large text and placeholders: 3:1. */
export const NON_TEXT = 3;

/** The seven base surfaces of each interface theme. */
export const BASE_SURFACES = [
  '--bg-base',
  '--bg-sidebar',
  '--bg-surface',
  '--bg-raised',
  '--bg-raised-hover',
  '--bg-input',
  '--bg-overlay',
] as const;

/** Surfaces that exist only while an item is pressed or hovered in a menu. */
export const STATE_SURFACES = ['--bg-pressed', '--bg-overlay-hover'] as const;

export const ALL_SURFACES = [...BASE_SURFACES, ...STATE_SURFACES] as const;

export const SUBTLE_BACKGROUNDS = [
  '--accent-subtle',
  '--danger-subtle',
  '--warning-subtle',
  '--success-subtle',
  '--info-subtle',
] as const;

export const TAG_COLORS = [
  '--tag-graphite',
  '--tag-jade',
  '--tag-amber',
  '--tag-coral',
  '--tag-plum',
  '--tag-moss',
  '--tag-ocean',
  '--tag-rose',
] as const;

export interface UiContrastRule {
  readonly name: string;
  readonly threshold: number;
  /** Token names of the foreground colors. */
  readonly foregrounds: readonly string[];
  /** Token names of the background colors. */
  readonly backgrounds: readonly string[];
  /** Opaque surfaces under translucent backgrounds; every background is drawn over each of them. */
  readonly under?: readonly string[];
}

/** Interface pairs, checked in both themes. */
export const UI_CONTRAST_RULES: readonly UiContrastRule[] = [
  {
    name: 'text on base surfaces',
    threshold: TEXT,
    foregrounds: [
      '--text-primary',
      '--text-secondary',
      '--accent-text',
      '--danger',
      '--danger-hover',
      '--warning',
      '--success',
      '--info',
    ],
    backgrounds: BASE_SURFACES,
  },
  {
    name: 'text on state surfaces',
    threshold: TEXT,
    foregrounds: [
      '--text-primary',
      '--text-secondary',
      '--accent-text',
      '--danger-hover',
      '--warning',
      '--success',
      '--info',
    ],
    backgrounds: STATE_SURFACES,
  },
  {
    name: 'primary button',
    threshold: TEXT,
    foregrounds: ['--text-on-accent'],
    backgrounds: ['--accent', '--accent-hover', '--accent-pressed'],
  },
  {
    name: 'danger button',
    threshold: TEXT,
    foregrounds: ['--text-on-danger'],
    backgrounds: ['--danger', '--danger-hover', '--danger-pressed'],
  },
  {
    name: 'text on tinted backgrounds',
    threshold: TEXT,
    foregrounds: ['--text-primary'],
    backgrounds: SUBTLE_BACKGROUNDS,
    under: BASE_SURFACES,
  },
  {
    name: 'secondary text in a selection',
    threshold: TEXT,
    foregrounds: ['--text-secondary'],
    backgrounds: ['--accent-subtle'],
    under: ['--bg-sidebar', '--bg-surface', '--bg-raised', '--bg-overlay'],
  },
  {
    name: 'badge letters',
    threshold: TEXT,
    foregrounds: ['--text-on-tag'],
    backgrounds: ['--tag-jade', '--tag-plum', '--tag-amber'],
  },
  {
    name: 'muted text',
    threshold: NON_TEXT,
    foregrounds: ['--text-muted'],
    backgrounds: BASE_SURFACES,
  },
  {
    name: 'status dots',
    threshold: NON_TEXT,
    foregrounds: ['--success', '--warning', '--danger', '--text-muted'],
    backgrounds: BASE_SURFACES,
  },
  {
    name: 'focus ring and accent indicators',
    threshold: NON_TEXT,
    foregrounds: ['--focus-ring', '--accent'],
    backgrounds: ALL_SURFACES,
  },
  {
    name: 'accent icon on its tinted background',
    threshold: NON_TEXT,
    foregrounds: ['--accent'],
    backgrounds: ['--accent-subtle'],
    under: BASE_SURFACES,
  },
  {
    name: 'danger icon on its tinted background',
    threshold: NON_TEXT,
    foregrounds: ['--danger'],
    backgrounds: ['--danger-subtle'],
    under: BASE_SURFACES,
  },
  {
    name: 'warning icon on its tinted background',
    threshold: NON_TEXT,
    foregrounds: ['--warning'],
    backgrounds: ['--warning-subtle'],
    under: BASE_SURFACES,
  },
  {
    name: 'success icon on its tinted background',
    threshold: NON_TEXT,
    foregrounds: ['--success'],
    backgrounds: ['--success-subtle'],
    under: BASE_SURFACES,
  },
  {
    name: 'info icon on its tinted background',
    threshold: NON_TEXT,
    foregrounds: ['--info'],
    backgrounds: ['--info-subtle'],
    under: BASE_SURFACES,
  },
  {
    name: 'tile icons',
    threshold: NON_TEXT,
    foregrounds: ['--text-on-tag'],
    backgrounds: TAG_COLORS,
  },
  {
    name: 'progress',
    threshold: NON_TEXT,
    foregrounds: ['--accent'],
    backgrounds: ['--border-subtle'],
  },
];

/**
 * A color of a terminal theme:
 * - a theme field (`foreground`, `cursor`, ...); `selection` is drawn over `background`;
 * - `selectedText`: `selectionForeground`, or `foreground` when it is absent;
 * - `ansi.<name>`: one of the 16 ANSI colors;
 * - `keyword.<category>`: the default highlighting color of the column chosen for `background`.
 */
export type TerminalColorRef =
  | 'background'
  | 'foreground'
  | 'cursor'
  | 'cursorText'
  | 'selection'
  | 'selectedText'
  | 'searchMatch'
  | 'searchMatchActive'
  | `ansi.${AnsiColorName}`
  | `keyword.${KeywordCategory}`;

export interface TerminalContrastRule {
  readonly name: string;
  readonly threshold: number;
  readonly foregrounds: readonly TerminalColorRef[];
  readonly backgrounds: readonly TerminalColorRef[];
}

/** Terminal pairs, checked for every built-in theme in ./terminal/. */
export const TERMINAL_CONTRAST_RULES: readonly TerminalContrastRule[] = [
  {
    name: 'text',
    threshold: TEXT,
    foregrounds: ['foreground'],
    backgrounds: ['background'],
  },
  {
    name: 'ANSI colors 1-6 and 9-14',
    threshold: TEXT,
    foregrounds: [
      'ansi.red',
      'ansi.green',
      'ansi.yellow',
      'ansi.blue',
      'ansi.magenta',
      'ansi.cyan',
      'ansi.brightRed',
      'ansi.brightGreen',
      'ansi.brightYellow',
      'ansi.brightBlue',
      'ansi.brightMagenta',
      'ansi.brightCyan',
    ],
    backgrounds: ['background'],
  },
  {
    name: 'bright black',
    threshold: NON_TEXT,
    foregrounds: ['ansi.brightBlack'],
    backgrounds: ['background'],
  },
  {
    name: 'selected text',
    threshold: TEXT,
    foregrounds: ['selectedText'],
    backgrounds: ['selection'],
  },
  {
    name: 'text under the cursor',
    threshold: TEXT,
    foregrounds: ['cursorText'],
    backgrounds: ['cursor'],
  },
  {
    name: 'cursor',
    threshold: NON_TEXT,
    foregrounds: ['cursor'],
    backgrounds: ['background'],
  },
  {
    name: 'search matches',
    threshold: TEXT,
    foregrounds: ['foreground'],
    backgrounds: ['searchMatch', 'searchMatchActive'],
  },
  {
    name: 'keyword highlighting',
    threshold: TEXT,
    foregrounds: [
      'keyword.error',
      'keyword.warning',
      'keyword.ok',
      'keyword.info',
      'keyword.debug',
      'keyword.address',
    ],
    backgrounds: ['background'],
  },
];
