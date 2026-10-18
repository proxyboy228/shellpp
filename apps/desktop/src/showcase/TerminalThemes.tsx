// Previews of the built-in terminal themes (docs/design-system.md, 4 and 7.2).
// Colors come from the theme files in src/theme/terminal/, not from this file.

import { Fragment, type CSSProperties } from 'react';
import { BUILTIN_TERMINAL_THEMES, DEFAULT_TERMINAL_THEME_NAME } from '../theme/builtin-terminal-themes';
import { isLightBackground, parseColor } from '../theme/color';
import { defaultKeywordColors, type KeywordCategory, type KeywordColors } from '../theme/keyword-highlight';
import { ANSI_COLOR_NAMES, type AnsiColorName, type TerminalTheme } from '../theme/terminal-theme';
import { Section } from './Section';
import styles from './TerminalThemes.module.css';

type Highlight = 'selection' | 'searchMatch' | 'searchMatchActive' | 'cursor';

interface Segment {
  readonly text: string;
  /** ANSI text color; the theme foreground when absent. */
  readonly ansi?: AnsiColorName;
  /** Keyword highlighting color (docs/design-system.md, 4.3). */
  readonly keyword?: KeywordCategory;
  readonly bold?: boolean;
  readonly highlight?: Highlight;
}

function prompt(command: string): Segment[] {
  return [
    { text: 'deploy@web-1', ansi: 'green', bold: true },
    { text: ':' },
    { text: '~/app', ansi: 'blue', bold: true },
    { text: ' (main)', ansi: 'yellow' },
    { text: `$ ${command}` },
  ];
}

/** A short sample session that uses the ANSI colors and every highlight. */
const SCREEN: readonly (readonly Segment[])[] = [
  prompt('ls'),
  [
    { text: 'bin', ansi: 'blue', bold: true },
    { text: '  ' },
    { text: 'logs', ansi: 'cyan', bold: true },
    { text: '  ' },
    { text: 'deploy.sh', ansi: 'green', bold: true },
    { text: '  ' },
    { text: 'backup.tar.gz', ansi: 'red', bold: true },
    { text: '  ' },
    { text: 'logo.png', ansi: 'magenta', bold: true },
    { text: '  README.md' },
  ],
  prompt('tail -n 5 logs/app.log'),
  [
    { text: '12:04:01 ' },
    { text: 'ERROR', keyword: 'error' },
    { text: ' upstream ' },
    { text: '10.0.4.17', keyword: 'address' },
    { text: ' refused the connection' },
  ],
  [{ text: '12:04:02 ' }, { text: 'WARNING', keyword: 'warning' }, { text: ' retrying in 5 s' }],
  [{ text: '12:04:07 ' }, { text: 'OK', keyword: 'ok' }, { text: ' connected to ' }, { text: '10.0.4.17', keyword: 'address' }],
  [{ text: '12:04:07 ' }, { text: 'INFO', keyword: 'info' }, { text: ' cache warmed in 212 ms' }],
  [{ text: '12:04:08 ' }, { text: 'DEBUG', keyword: 'debug' }, { text: ' pool size 8' }],
  prompt('grep -n web-1 deploy.sh'),
  [
    { text: '3', ansi: 'green' },
    { text: ':rsync -a ./build/ deploy@' },
    { text: 'web-1', highlight: 'searchMatchActive' },
    { text: ':/srv/app/' },
  ],
  [
    { text: '4', ansi: 'green' },
    { text: ':ssh deploy@' },
    { text: 'web-1', highlight: 'searchMatch' },
    { text: ' ' },
    { text: 'systemctl restart app', highlight: 'selection' },
  ],
  [...prompt(''), { text: ' ', highlight: 'cursor' }],
];

function segmentStyle(theme: TerminalTheme, keywords: KeywordColors, segment: Segment): CSSProperties {
  const style: CSSProperties = {};
  if (segment.ansi !== undefined) {
    style.color = theme.ansi[segment.ansi];
  }
  if (segment.keyword !== undefined) {
    style.color = keywords[segment.keyword];
  }
  switch (segment.highlight) {
    case 'selection':
      // Drawn over the background; text keeps its colors unless the theme sets one.
      style.background = theme.selection;
      if (theme.selectionForeground !== undefined) {
        style.color = theme.selectionForeground;
      }
      break;
    case 'searchMatch':
      style.background = theme.searchMatch;
      style.color = theme.foreground;
      break;
    case 'searchMatchActive':
      style.background = theme.searchMatchActive;
      style.color = theme.foreground;
      break;
    case 'cursor':
      style.background = theme.cursor;
      style.color = theme.cursorText;
      break;
    default:
      break;
  }
  return style;
}

interface RoleSpec {
  readonly role: string;
  readonly color: string;
}

function themeRoles(theme: TerminalTheme): RoleSpec[] {
  const roles: RoleSpec[] = [
    { role: 'background', color: theme.background },
    { role: 'foreground', color: theme.foreground },
    { role: 'cursor', color: theme.cursor },
    { role: 'cursorText', color: theme.cursorText },
    { role: 'selection', color: theme.selection },
  ];
  if (theme.selectionForeground !== undefined) {
    roles.push({ role: 'selectionForeground', color: theme.selectionForeground });
  }
  roles.push(
    { role: 'searchMatch', color: theme.searchMatch },
    { role: 'searchMatchActive', color: theme.searchMatchActive },
  );
  return roles;
}

function themeSlug(theme: TerminalTheme): string {
  return theme.name.toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

function TerminalPreview({ theme }: { theme: TerminalTheme }) {
  const lightBackground = isLightBackground(parseColor(theme.background));
  const keywords = defaultKeywordColors(theme.background);
  const isDefault = theme.name === DEFAULT_TERMINAL_THEME_NAME;
  const slug = themeSlug(theme);
  // Frames of the color chips: bright black stays visible on the theme background.
  const chipBorder: CSSProperties = { borderColor: theme.ansi.brightBlack };

  return (
    <figure
      className={styles.theme}
      style={{ background: theme.background, color: theme.foreground }}
      aria-labelledby={`${slug}-name`}
      data-testid={`terminal-theme-${slug}`}
    >
      <figcaption className={styles.header}>
        <span id={`${slug}-name`} className={styles.name}>
          {theme.name}
        </span>
        <span className={styles.badge} style={chipBorder}>
          {lightBackground ? 'Light background' : 'Dark background'}
          {isDefault ? ', default' : ''}
        </span>
      </figcaption>

      <pre className={styles.screen} style={chipBorder}>
        {SCREEN.map((line, lineIndex) => (
          <Fragment key={lineIndex}>
            {line.map((segment, segmentIndex) => (
              <span
                key={segmentIndex}
                className={segment.bold ? styles.bold : undefined}
                style={segmentStyle(theme, keywords, segment)}
              >
                {segment.text}
              </span>
            ))}
            {'\n'}
          </Fragment>
        ))}
        <span aria-hidden="true">
          {ANSI_COLOR_NAMES.slice(0, 8).map((name) => (
            <span key={name} className={styles.block} style={{ background: theme.ansi[name] }}>
              {'   '}
            </span>
          ))}
          {'\n'}
          {ANSI_COLOR_NAMES.slice(8).map((name) => (
            <span key={name} className={styles.block} style={{ background: theme.ansi[name] }}>
              {'   '}
            </span>
          ))}
        </span>
      </pre>

      <h4 id={`${slug}-ansi`} className={styles.listTitle}>
        ANSI colors
      </h4>
      <ol className={styles.colors} aria-labelledby={`${slug}-ansi`}>
        {ANSI_COLOR_NAMES.map((name, index) => (
          <li key={name} className={styles.color}>
            <span className={styles.chip} style={{ ...chipBorder, background: theme.ansi[name] }} aria-hidden="true" />
            <span className={styles.colorText}>
              <span className={styles.colorName}>
                {index} {name}
              </span>
              <code className={styles.colorValue}>{theme.ansi[name]}</code>
            </span>
          </li>
        ))}
      </ol>

      <h4 id={`${slug}-roles`} className={styles.listTitle}>
        Roles
      </h4>
      <ul className={styles.colors} aria-labelledby={`${slug}-roles`}>
        {themeRoles(theme).map(({ role, color }) => (
          <li key={role} className={styles.color}>
            <span className={styles.chip} style={{ ...chipBorder, background: color }} aria-hidden="true" />
            <span className={styles.colorText}>
              <span className={styles.colorName}>{role}</span>
              <code className={styles.colorValue}>{color}</code>
            </span>
          </li>
        ))}
      </ul>
    </figure>
  );
}

export function TerminalThemes() {
  return (
    <Section
      id="terminal"
      title="Terminal themes"
      description="Built-in themes from src/theme/terminal/. The terminal theme is chosen per host and does not follow the interface theme. The log lines use the default keyword highlighting colors for the theme background."
    >
      <div className={styles.grid}>
        {BUILTIN_TERMINAL_THEMES.map((theme) => (
          <TerminalPreview key={theme.name} theme={theme} />
        ))}
      </div>
    </Section>
  );
}
