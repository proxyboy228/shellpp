// Interface theme choice for the showcase (docs/design-system.md, 7.1).
// Light and Dark set `data-theme` on <html>; Auto removes it, so that
// tokens.css follows `prefers-color-scheme`.

import { useSyncExternalStore } from 'react';
import type { ThemeName } from '../theme/tokens';

export type ThemeChoice = 'auto' | ThemeName;

export const THEME_ATTRIBUTE = 'data-theme';

const LIGHT_SCHEME_QUERY = '(prefers-color-scheme: light)';

/** Reads the choice from the `data-theme` attribute; anything else counts as Auto. */
export function readThemeChoice(root: HTMLElement = document.documentElement): ThemeChoice {
  const value = root.getAttribute(THEME_ATTRIBUTE);
  return value === 'light' || value === 'dark' ? value : 'auto';
}

/** Sets `data-theme` for Light and Dark, removes it for Auto. */
export function applyThemeChoice(choice: ThemeChoice, root: HTMLElement = document.documentElement): void {
  if (choice === 'auto') {
    root.removeAttribute(THEME_ATTRIBUTE);
  } else {
    root.setAttribute(THEME_ATTRIBUTE, choice);
  }
}

/** The theme that is in effect for a choice and a system theme. */
export function resolveThemeChoice(choice: ThemeChoice, system: ThemeName): ThemeName {
  return choice === 'auto' ? system : choice;
}

function subscribeToSystemTheme(onChange: () => void): () => void {
  const query = window.matchMedia(LIGHT_SCHEME_QUERY);
  query.addEventListener('change', onChange);
  return () => query.removeEventListener('change', onChange);
}

function getSystemTheme(): ThemeName {
  return window.matchMedia(LIGHT_SCHEME_QUERY).matches ? 'light' : 'dark';
}

function getServerSystemTheme(): ThemeName {
  return 'dark';
}

/**
 * The system theme as tokens.css sees it: light only when the system asks
 * for a light scheme, dark otherwise (dark is the default).
 */
export function useSystemTheme(): ThemeName {
  return useSyncExternalStore(subscribeToSystemTheme, getSystemTheme, getServerSystemTheme);
}
