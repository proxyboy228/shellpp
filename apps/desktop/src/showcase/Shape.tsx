// Spacing, radii and control sizes (docs/design-system.md, 6).

import styles from './Shape.module.css';
import { Section, Specimen } from './Section';
import { sharedTokenValue } from './token-values';

const SPACING = ['--space-1', '--space-2', '--space-3', '--space-4', '--space-5', '--space-6'];

const RADII: readonly { token: string; usage: string }[] = [
  { token: '--radius-control', usage: 'Buttons, fields, chips' },
  { token: '--radius-tile', usage: 'Icon tiles' },
  { token: '--radius-card', usage: 'Cards, panel blocks' },
  { token: '--radius-overlay', usage: 'Menus, modal windows' },
];

const HEIGHTS: readonly { token: string; usage: string }[] = [
  { token: '--height-control', usage: 'Buttons and fields' },
  { token: '--height-top-bar', usage: 'Top bar' },
  { token: '--height-row', usage: 'Table rows' },
  { token: '--height-host-card', usage: 'Host cards' },
];

function Token({ token, usage }: { token: string; usage?: string }) {
  return (
    <span className={styles.label}>
      <code className={styles.token}>{token}</code>
      <span className={styles.caption}>
        {sharedTokenValue(token)}
        {usage === undefined ? null : `, ${usage}`}
      </span>
    </span>
  );
}

export function Shape() {
  return (
    <Section id="shape" title="Spacing and shape" description="A 4 px grid, four radii and fixed control heights.">
      <Specimen id="shape-spacing" title="Spacing">
        <ul className={styles.list}>
          {SPACING.map((token) => (
            <li key={token} className={styles.item}>
              <Token token={token} />
              <span className={styles.bar} style={{ width: `var(${token})` }} aria-hidden="true" />
            </li>
          ))}
        </ul>
      </Specimen>

      <Specimen id="shape-radii" title="Radii">
        <ul className={styles.tiles}>
          {RADII.map(({ token, usage }) => (
            <li key={token} className={styles.tileItem}>
              <span className={styles.radius} style={{ borderRadius: `var(${token})` }} aria-hidden="true" />
              <Token token={token} usage={usage} />
            </li>
          ))}
        </ul>
      </Specimen>

      <Specimen id="shape-heights" title="Heights">
        <ul className={styles.list}>
          {HEIGHTS.map(({ token, usage }) => (
            <li key={token} className={styles.item}>
              <Token token={token} usage={usage} />
              <span className={styles.height} style={{ height: `var(${token})` }} aria-hidden="true" />
            </li>
          ))}
        </ul>
      </Specimen>
    </Section>
  );
}
