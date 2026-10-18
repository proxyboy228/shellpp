// The stage 0 showcase page: color tokens, typography, spacing, terminal
// themes and every base component, with an Auto / Light / Dark switch
// (docs/decisions.md 1, docs/design-system.md 7.1).

import { Monitor, Moon, Sun } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { SegmentedControl, type SegmentedControlOption } from '../components';
import styles from './App.module.css';
import { ColorTokens } from './ColorTokens';
import { COMPONENT_SPECIMENS, Components } from './Components';
import { Shape } from './Shape';
import { TerminalThemes } from './TerminalThemes';
import { applyThemeChoice, readThemeChoice, resolveThemeChoice, useSystemTheme, type ThemeChoice } from './theme';
import { Typography } from './Typography';

export const SHOWCASE_TITLE = 'Shell++ design system';

const THEME_OPTIONS: readonly SegmentedControlOption<ThemeChoice>[] = [
  { value: 'auto', label: 'Auto', icon: <Monitor /> },
  { value: 'light', label: 'Light', icon: <Sun /> },
  { value: 'dark', label: 'Dark', icon: <Moon /> },
];

interface NavigationItem {
  readonly id: string;
  readonly title: string;
  readonly children?: readonly { id: string; title: string }[];
}

const NAVIGATION: readonly NavigationItem[] = [
  { id: 'colors', title: 'Colors' },
  { id: 'typography', title: 'Typography' },
  { id: 'shape', title: 'Spacing and shape' },
  { id: 'terminal', title: 'Terminal themes' },
  { id: 'components', title: 'Components', children: COMPONENT_SPECIMENS },
];

export function App() {
  const [themeChoice, setThemeChoice] = useState<ThemeChoice>(() => readThemeChoice());
  const [lastAction, setLastAction] = useState<string | null>(null);
  const systemTheme = useSystemTheme();
  const theme = resolveThemeChoice(themeChoice, systemTheme);
  const themeLabelId = useId();

  useEffect(() => {
    applyThemeChoice(themeChoice);
  }, [themeChoice]);

  useEffect(() => {
    document.title = SHOWCASE_TITLE;
  }, []);

  return (
    <div className={styles.page}>
      <header className={styles.topBar}>
        <h1 className={styles.title}>{SHOWCASE_TITLE}</h1>
        <p className={styles.lastAction} role="status" data-testid="showcase-last-action">
          Last action: {lastAction ?? 'none'}
        </p>
        <div className={styles.themeSwitch}>
          <span id={themeLabelId} className={styles.themeLabel}>
            Theme
          </span>
          <SegmentedControl
            aria-labelledby={themeLabelId}
            options={THEME_OPTIONS}
            value={themeChoice}
            onValueChange={setThemeChoice}
            data-testid="showcase-theme"
          />
          <span className={styles.themeNote} data-testid="showcase-effective-theme">
            {themeChoice === 'auto' ? `System: ${theme}` : null}
          </span>
        </div>
      </header>

      <div className={styles.layout}>
        <nav className={styles.nav} aria-label="Sections">
          <ul className={styles.navList}>
            {NAVIGATION.map((item) => (
              <li key={item.id}>
                <a className={styles.navLink} href={`#${item.id}`}>
                  {item.title}
                </a>
                {item.children === undefined ? null : (
                  <ul className={styles.navSubList}>
                    {item.children.map((child) => (
                      <li key={child.id}>
                        <a className={`${styles.navLink} ${styles.navSubLink}`} href={`#${child.id}`}>
                          {child.title}
                        </a>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        </nav>

        <main className={styles.content}>
          <ColorTokens theme={theme} />
          <Typography />
          <Shape />
          <TerminalThemes />
          <Components onAction={setLastAction} />
        </main>
      </div>
    </div>
  );
}

export default App;
