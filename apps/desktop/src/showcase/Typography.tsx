// Typography samples (docs/design-system.md, 5).

import styles from './Typography.module.css';
import { Section, Specimen } from './Section';
import { sharedTokenValue } from './token-values';

const UI_SAMPLE = 'Sphinx of black quartz, judge my vow. 0123456789';
const MONO_SAMPLE = 'ssh -p 2222 deploy@10.0.4.17 -- tail -f /var/log/app.log';

interface FontSizeSpec {
  readonly token: string;
  readonly usage: string;
  readonly mono?: boolean;
}

const FONT_SIZES: readonly FontSizeSpec[] = [
  { token: '--font-size-caption', usage: 'Card subtitles, captions' },
  { token: '--font-size-dense', usage: 'Dense lists and tables' },
  { token: '--font-size-body', usage: 'Body text' },
  { token: '--font-size-heading', usage: 'Section headings: Groups, Hosts, Keys' },
  { token: '--font-size-title', usage: 'Modal window titles' },
  { token: '--font-size-mono', usage: 'Terminal, script and key fields', mono: true },
];

const FONT_WEIGHTS: readonly { token: string; name: string }[] = [
  { token: '--font-weight-regular', name: 'Regular' },
  { token: '--font-weight-medium', name: 'Medium' },
  { token: '--font-weight-semibold', name: 'Semibold' },
];

const TABLE_ROWS: readonly { name: string; size: string; modified: string }[] = [
  { name: 'backup.tar.gz', size: '1,118.40 MB', modified: '2026-10-11 09:41' },
  { name: 'notes.txt', size: '4.07 KB', modified: '2026-10-17 18:05' },
  { name: 'deploy.sh', size: '911 B', modified: '2026-10-18 07:30' },
];

function FontStack({ token, sample, mono = false }: { token: string; sample: string; mono?: boolean }) {
  return (
    <div className={styles.family}>
      <p className={mono ? styles.monoSample : styles.uiSample}>{sample}</p>
      <p className={styles.caption}>
        <code className={styles.token}>{token}</code> {sharedTokenValue(token)}
      </p>
    </div>
  );
}

export function Typography() {
  return (
    <Section
      id="typography"
      title="Typography"
      description="The system interface font with a monospace font for the terminal. Stage 0 does not bundle fonts yet; the stacks fall back to installed fonts."
    >
      <Specimen id="type-families" title="Font families">
        <FontStack token="--font-ui" sample={UI_SAMPLE} />
        <FontStack token="--font-mono" sample={MONO_SAMPLE} mono />
      </Specimen>

      <Specimen id="type-sizes" title="Sizes">
        <ul className={styles.list}>
          {FONT_SIZES.map(({ token, usage, mono }) => (
            <li key={token} className={styles.sizeRow}>
              <div className={styles.meta}>
                <code className={styles.token}>{token}</code>
                <span className={styles.caption}>
                  {sharedTokenValue(token)}, {usage}
                </span>
              </div>
              <p className={mono ? styles.monoSample : styles.uiSample} style={{ fontSize: `var(${token})` }}>
                {mono ? MONO_SAMPLE : UI_SAMPLE}
              </p>
            </li>
          ))}
        </ul>
      </Specimen>

      <Specimen id="type-weights" title="Weights">
        <ul className={styles.list}>
          {FONT_WEIGHTS.map(({ token, name }) => (
            <li key={token} className={styles.sizeRow}>
              <div className={styles.meta}>
                <code className={styles.token}>{token}</code>
                <span className={styles.caption}>
                  {sharedTokenValue(token)}, {name}
                </span>
              </div>
              <p className={styles.uiSample} style={{ fontWeight: `var(${token})` }}>
                {name}: {UI_SAMPLE}
              </p>
            </li>
          ))}
        </ul>
      </Specimen>

      <Specimen
        id="type-numbers"
        title="Numbers in tables"
        description="Sizes, dates and times use tabular figures so that columns line up."
      >
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col" className={styles.numeric}>
                Size
              </th>
              <th scope="col" className={styles.numeric}>
                Modified
              </th>
            </tr>
          </thead>
          <tbody>
            {TABLE_ROWS.map((row) => (
              <tr key={row.name}>
                <td>{row.name}</td>
                <td className={styles.numeric}>{row.size}</td>
                <td className={styles.numeric}>{row.modified}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Specimen>

      <Specimen
        id="type-mono"
        title="Monospace block"
        description={`Line height ${sharedTokenValue('--line-height-mono')} (--line-height-mono), as in the terminal and script fields.`}
      >
        <pre className={styles.codeBlock}>
          {['#!/bin/sh', 'set -eu', 'rsync -a ./build/ deploy@web-1:/srv/app/', 'ssh deploy@web-1 systemctl restart app'].join(
            '\n',
          )}
        </pre>
      </Specimen>
    </Section>
  );
}
