// Terminal theme of a host (UI 3.1 "Show more"): a preview thumbnail with the
// theme name; the picker offers Default (the global theme), Graphite and Paper.

import { useRef, useState, type CSSProperties } from 'react';
import type { ThemeRef } from '../../../ipc';
import { t } from '../../../i18n';
import { BUILTIN_TERMINAL_THEME_FILES } from '../../../theme/builtin-terminal-themes';
import type { TerminalTheme } from '../../../theme/terminal-theme';
import { Popover } from '../Popover';
import type { ThemeChoice } from '../fields';
import styles from '../panels.module.css';

/** Built-in themes by id (the file name: `graphite`, `paper`). */
export const BUILTIN_THEMES: ReadonlyMap<string, TerminalTheme> = new Map(
  [...BUILTIN_TERMINAL_THEME_FILES].map(([path, theme]) => [path.replace(/^.*\/(.+)\.json$/, '$1'), theme]),
);

const FALLBACK_THEME_ID = 'graphite';
const SWATCHES = ['red', 'green', 'yellow', 'blue', 'magenta', 'cyan'] as const;

function themeFor(ref: ThemeRef | null | undefined): TerminalTheme | undefined {
  if (ref !== null && ref !== undefined && 'builtin' in ref) {
    return BUILTIN_THEMES.get(ref.builtin);
  }
  return undefined;
}

function ThemeThumbnail({ theme }: { theme: TerminalTheme | undefined }) {
  if (theme === undefined) {
    return <span className={styles.themeThumb} aria-hidden="true" />;
  }
  // Theme colors are data (the theme files), applied through custom properties.
  const style = { '--thumb-bg': theme.background, '--thumb-fg': theme.foreground } as CSSProperties;
  return (
    <span className={styles.themeThumb} style={style} aria-hidden="true">
      <span className={styles.themeThumbText} />
      <span className={styles.themeSwatches}>
        {SWATCHES.map((name) => (
          <span
            key={name}
            className={styles.themeSwatch}
            style={{ '--swatch': theme.ansi[name] } as CSSProperties}
          />
        ))}
      </span>
    </span>
  );
}

export interface ThemePickerProps {
  value: ThemeChoice;
  onChange: (theme: ThemeChoice) => void;
  /** Setting `terminal.default_theme`, shown for Default. */
  defaultTheme: ThemeRef | null;
  /** Test id of the button; picker `<id>-picker`, options `<id>-option-<default|graphite|paper>`. */
  'data-testid': string;
}

export function ThemePicker({ value, onChange, defaultTheme, 'data-testid': testId }: ThemePickerProps) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const globalTheme = themeFor(defaultTheme) ?? BUILTIN_THEMES.get(FALLBACK_THEME_ID);
  const defaultLabel = t('host_panel.theme.default', { name: globalTheme?.name ?? '' });

  let current: TerminalTheme | undefined;
  let currentLabel: string;
  if (value === null) {
    current = globalTheme;
    currentLabel = defaultLabel;
  } else if ('builtin' in value) {
    current = BUILTIN_THEMES.get(value.builtin);
    currentLabel = current?.name ?? t('host_panel.theme.custom');
  } else {
    current = undefined;
    currentLabel = t('host_panel.theme.custom');
  }

  const choose = (choice: ThemeChoice) => {
    onChange(choice);
    setOpen(false);
    buttonRef.current?.focus();
  };

  const selectedId = value === null ? 'default' : 'builtin' in value ? value.builtin : '';
  const options: { id: string; label: string; theme: TerminalTheme | undefined; choice: ThemeChoice }[] = [
    { id: 'default', label: defaultLabel, theme: globalTheme, choice: null },
    ...[...BUILTIN_THEMES].map(([id, theme]) => ({ id, label: theme.name, theme, choice: { builtin: id } })),
  ];

  return (
    <div className={styles.field}>
      <span className={styles.label} id={`${testId}-label`}>
        {t('host_panel.theme.label')}
      </span>
      <button
        ref={buttonRef}
        type="button"
        className={styles.themeButton}
        aria-labelledby={`${testId}-label ${testId}-name`}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((isOpen) => !isOpen)}
        data-testid={testId}
      >
        <ThemeThumbnail theme={current} />
        <span id={`${testId}-name`} className={styles.pickerValue}>
          {currentLabel}
        </span>
      </button>
      {open ? (
        <Popover
          anchorRef={buttonRef}
          onClose={() => setOpen(false)}
          placement="inline"
          role="listbox"
          aria-label={t('host_panel.theme.label')}
          data-testid={`${testId}-picker`}
        >
          {options.map((option) => (
            <div
              key={option.id}
              role="option"
              tabIndex={0}
              aria-selected={option.id === selectedId}
              className={styles.themeOption}
              onClick={() => choose(option.choice)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  choose(option.choice);
                }
              }}
              data-testid={`${testId}-option-${option.id}`}
            >
              <ThemeThumbnail theme={option.theme} />
              <span className={styles.optionLabel}>{option.label}</span>
            </div>
          ))}
        </Popover>
      ) : null}
    </div>
  );
}

export default ThemePicker;
