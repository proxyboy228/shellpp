// Color tokens as swatches (docs/design-system.md, 2 and 3). Every sample is
// painted with `var(--token)`; the caption shows the value in the theme that
// is in effect.

import type { CSSProperties } from 'react';
import type { ThemeName } from '../theme/tokens';
import styles from './ColorTokens.module.css';
import { Section } from './Section';
import { tokenValue } from './token-values';

/**
 * How a token is drawn: a filled area, sample text, a focus outline or a
 * shadow under a raised box.
 */
type SampleKind = 'fill' | 'text' | 'ring' | 'shadow';

interface ColorTokenSpec {
  readonly name: string;
  readonly purpose: string;
  /** Default: `fill`. */
  readonly kind?: SampleKind;
  /** Background token of a text sample. Default: the sample surface. */
  readonly on?: string;
}

interface ColorTokenGroup {
  readonly id: string;
  readonly title: string;
  readonly note?: string;
  readonly tokens: readonly ColorTokenSpec[];
}

/** Every color and shadow token of tokens.css, grouped as in design-system.md. */
export const COLOR_TOKEN_GROUPS: readonly ColorTokenGroup[] = [
  {
    id: 'surfaces',
    title: 'Surfaces',
    tokens: [
      { name: '--bg-base', purpose: 'Window background, top bar with tabs' },
      { name: '--bg-sidebar', purpose: 'Sidebar' },
      { name: '--bg-surface', purpose: 'Main content area' },
      { name: '--bg-raised', purpose: 'Cards, details panel blocks' },
      { name: '--bg-raised-hover', purpose: 'Hover over a card, row or button' },
      { name: '--bg-pressed', purpose: 'Pressed card, row or button' },
      { name: '--bg-input', purpose: 'Input fields inside blocks' },
      { name: '--bg-overlay', purpose: 'Menus, dropdowns, modal windows, toasts' },
      { name: '--bg-overlay-hover', purpose: 'Hover over a menu or dropdown item' },
      { name: '--scrim', purpose: 'Dimming under a modal window' },
    ],
  },
  {
    id: 'borders',
    title: 'Borders',
    tokens: [
      { name: '--border-subtle', purpose: 'Dividers, card borders, progress track' },
      { name: '--border-strong', purpose: 'Field borders, menu dividers, chip borders, off toggle track' },
    ],
  },
  {
    id: 'text',
    title: 'Text',
    tokens: [
      { name: '--text-primary', purpose: 'Primary text', kind: 'text' },
      { name: '--text-secondary', purpose: 'Card subtitles, captions, table columns, icons', kind: 'text' },
      { name: '--text-muted', purpose: 'Placeholders, secondary captions', kind: 'text' },
      { name: '--text-disabled', purpose: 'Text of inactive menu items only', kind: 'text' },
    ],
  },
  {
    id: 'accent',
    title: 'Accent',
    tokens: [
      { name: '--accent', purpose: 'Primary button, selection, enabled toggle, active tab bar, progress' },
      { name: '--accent-hover', purpose: 'Accent under the pointer' },
      { name: '--accent-pressed', purpose: 'Pressed accent' },
      { name: '--accent-subtle', purpose: 'Active navigation item, selected row, active filter chip' },
      { name: '--accent-text', purpose: 'Links and text buttons', kind: 'text' },
      { name: '--text-on-accent', purpose: 'Text on an accent background', kind: 'text', on: '--accent' },
      { name: '--focus-ring', purpose: 'Keyboard focus outline', kind: 'ring' },
    ],
  },
  {
    id: 'semantic',
    title: 'Semantic colors',
    tokens: [
      { name: '--danger', purpose: 'Delete, Remove, Close in menus, errors' },
      { name: '--danger-hover', purpose: 'Dangerous action under the pointer' },
      { name: '--danger-pressed', purpose: 'Pressed danger button' },
      { name: '--danger-subtle', purpose: 'Background of blocking warnings' },
      { name: '--text-on-danger', purpose: 'Text on a danger background', kind: 'text', on: '--danger' },
      { name: '--warning', purpose: 'Warnings, connecting' },
      { name: '--warning-subtle', purpose: 'Warning background' },
      { name: '--success', purpose: 'Connected, rule running' },
      { name: '--success-subtle', purpose: 'Success background' },
      { name: '--info', purpose: 'Informational hints' },
      { name: '--info-subtle', purpose: 'Background of informational hints' },
    ],
  },
  {
    id: 'shadows',
    title: 'Shadows',
    tokens: [
      { name: '--shadow-overlay', purpose: 'Menus, popovers, toasts', kind: 'shadow' },
      { name: '--shadow-modal', purpose: 'Modal windows', kind: 'shadow' },
    ],
  },
  {
    id: 'tiles',
    title: 'Host and group tiles',
    note: 'The same in both themes.',
    tokens: [
      { name: '--tag-graphite', purpose: 'Default for hosts and groups' },
      { name: '--tag-jade', purpose: 'Tile; Local forwarding badge' },
      { name: '--tag-amber', purpose: 'Tile; Dynamic forwarding badge' },
      { name: '--tag-coral', purpose: 'Tile' },
      { name: '--tag-plum', purpose: 'Tile; Remote forwarding badge' },
      { name: '--tag-moss', purpose: 'Tile' },
      { name: '--tag-ocean', purpose: 'Tile' },
      { name: '--tag-rose', purpose: 'Tile' },
      { name: '--text-on-tag', purpose: 'Icons on tiles, letters on badges', kind: 'text', on: '--tag-jade' },
    ],
  },
];

function cssVar(name: string): string {
  return `var(${name})`;
}

function sampleStyle(token: ColorTokenSpec): CSSProperties | undefined {
  switch (token.kind ?? 'fill') {
    case 'text':
      return { color: cssVar(token.name), background: token.on === undefined ? undefined : cssVar(token.on) };
    case 'ring':
      return { outlineColor: cssVar(token.name) };
    case 'shadow':
      return { boxShadow: cssVar(token.name) };
    default:
      return undefined;
  }
}

const SAMPLE_KIND_CLASS: Readonly<Record<SampleKind, string | undefined>> = {
  fill: undefined,
  text: undefined,
  ring: styles.ring,
  shadow: styles.shadow,
};

function Swatch({ token, theme }: { token: ColorTokenSpec; theme: ThemeName }) {
  const kind = token.kind ?? 'fill';
  const sampleClassName = [styles.sample, SAMPLE_KIND_CLASS[kind]].filter(Boolean).join(' ');
  return (
    <li className={styles.swatch} data-testid={`swatch${token.name.slice(1)}`}>
      <div className={sampleClassName} style={sampleStyle(token)} aria-hidden="true">
        {kind === 'fill' ? <span className={styles.fillLayer} style={{ background: cssVar(token.name) }} /> : null}
        {kind === 'text' ? 'Aa' : null}
      </div>
      <code className={styles.name}>{token.name}</code>
      <code className={styles.value} data-testid={`swatch${token.name.slice(1)}-value`}>
        {tokenValue(theme, token.name)}
      </code>
      <p className={styles.purpose}>{token.purpose}</p>
    </li>
  );
}

interface ColorTokensProps {
  /** The theme in effect, for the value captions. */
  theme: ThemeName;
}

export function ColorTokens({ theme }: ColorTokensProps) {
  return (
    <Section
      id="colors"
      title="Colors"
      description={`Interface tokens from tokens.css. Values are shown for the ${theme} theme; switch the theme at the top of the page.`}
    >
      {COLOR_TOKEN_GROUPS.map((group) => (
        <div key={group.id} className={styles.group}>
          <h3 id={`colors-${group.id}`} className={styles.groupTitle}>
            {group.title}
            {group.note === undefined ? null : <span className={styles.groupNote}> {group.note}</span>}
          </h3>
          <ul className={styles.swatches} aria-labelledby={`colors-${group.id}`}>
            {group.tokens.map((token) => (
              <Swatch key={token.name} token={token} theme={theme} />
            ))}
          </ul>
        </div>
      ))}
    </Section>
  );
}
