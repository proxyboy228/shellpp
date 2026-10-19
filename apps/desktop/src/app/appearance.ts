// Interface theme and zoom from the settings (plan, "Settings keys used by
// the stage-1 UI"): `appearance.ui_theme` sets `data-theme` on `<html>` (Auto
// removes it, docs/design-system.md 7.1) and `appearance.ui_zoom` is applied
// as CSS `zoom` on `<html>` (View > Zoom, UI 1.4).
//
// Owner: W4.

import type { SettingsValues } from '../ipc';
import { enumSetting, numberSetting } from './store';

export const UI_ZOOM_KEY = 'appearance.ui_zoom';
export const UI_THEME_KEY = 'appearance.ui_theme';
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2;
export const ZOOM_STEP = 0.1;

/** Clamps and rounds a zoom factor to one decimal. */
export function normalizeZoom(zoom: number): number {
  const rounded = Math.round(zoom * 10) / 10;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, rounded));
}

export function applyAppearance(settings: SettingsValues, root: HTMLElement = document.documentElement): void {
  const theme = enumSetting(settings, UI_THEME_KEY, ['auto', 'light', 'dark'], 'auto');
  if (theme === 'auto') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', theme);
  }
  const zoom = normalizeZoom(numberSetting(settings, UI_ZOOM_KEY, 1));
  // Set through the CSSOM (allowed by the CSP, unlike inline style attributes).
  root.style.setProperty('zoom', String(zoom));
}
