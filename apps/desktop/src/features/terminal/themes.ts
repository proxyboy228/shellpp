// Terminal themes of stage 2 (stage 2 plan, decision 15): the built-in
// Graphite and Paper from src/theme/terminal/*.json. A session's
// `terminal_theme` is a built-in slug or a theme record; theme records
// (imported themes) arrive in stage 6, so a record reference shows Graphite.
//
// Owner: S6.

import { createStore, useStore } from '../../app/store/createStore';
import { BUILTIN_TERMINAL_THEME_FILES } from '../../theme/builtin-terminal-themes';
import type { TerminalTheme } from '../../theme/terminal-theme';
import type { ThemeRef, Uuid } from '../../ipc';
import { onWorkspaceReset } from './workspace';

export interface BuiltinTheme {
  /** File name without `.json`: `graphite`, `paper`. */
  slug: string;
  theme: TerminalTheme;
}

/** Built-in themes in file order (Graphite first). */
export const BUILTIN_THEMES: readonly BuiltinTheme[] = [...BUILTIN_TERMINAL_THEME_FILES.entries()].map(([path, theme]) => ({
  slug: path.replace(/^.*\//, '').replace(/\.json$/, ''),
  theme,
}));

export const DEFAULT_THEME_SLUG = 'graphite';

function builtin(slug: string): BuiltinTheme | undefined {
  return BUILTIN_THEMES.find((entry) => entry.slug === slug);
}

/** The theme to draw for a reference: a known built-in slug, else Graphite. */
export function resolveTheme(ref: ThemeRef | null | undefined): BuiltinTheme {
  const found = ref !== null && ref !== undefined && 'builtin' in ref ? builtin(ref.builtin) : undefined;
  return found ?? builtin(DEFAULT_THEME_SLUG) ?? (BUILTIN_THEMES[0] as BuiltinTheme);
}

/**
 * Themes chosen in the Appearance tab for running sessions (UI 4.2: "applies
 * the theme to the session"); saved hosts also store it, local and quick
 * sessions keep it only here.
 */
export const sessionThemeStore = createStore<ReadonlyMap<Uuid, ThemeRef>>(new Map());

onWorkspaceReset(() => sessionThemeStore.set(new Map()));

export function setSessionTheme(sessionId: Uuid, ref: ThemeRef): void {
  sessionThemeStore.set((current) => new Map(current).set(sessionId, ref));
}

/** The theme reference a session is drawn with: the Appearance choice, else the session's own. */
export function useSessionTheme(sessionId: Uuid, own: ThemeRef): BuiltinTheme {
  const overrides = useStore(sessionThemeStore);
  return resolveTheme(overrides.get(sessionId) ?? own);
}
