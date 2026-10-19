// Terminal settings read by the stage-2 UI (stage 2 plan, section 5.5). There
// is no Terminal settings screen until stage 6; the defaults below match
// docs/data-model.md 16.1 and UI 12.4. Font family, size and line height are
// written by the Appearance tab of the terminal sidebar.
//
// Owner: S6.

import { useMemo } from 'react';
import { enumSetting, numberSetting, useSettings } from '../../app/store';
import type { SettingsValues, ThemeRef } from '../../ipc';
import type { CursorStyle } from './engine/types';

export const FONT_SIZE_MIN = 6;
export const FONT_SIZE_MAX = 72;
export const LINE_HEIGHT_MIN = 1;
export const LINE_HEIGHT_MAX = 2;
export const DEFAULT_FONT_FAMILY = 'JetBrains Mono';

export interface TerminalSettings {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  cursorStyle: CursorStyle;
  cursorBlink: boolean;
  boldIsBright: boolean;
  copyOnSelect: boolean;
  bellSound: boolean;
  confirmMultilinePaste: boolean;
  autoreconnect: boolean;
  defaultTheme: ThemeRef;
}

function boolSetting(settings: SettingsValues, key: string, fallback: boolean): boolean {
  const value = settings[key];
  return typeof value === 'boolean' ? value : fallback;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** The effective terminal settings with defaults applied. */
export function readTerminalSettings(settings: SettingsValues): TerminalSettings {
  const family = settings['terminal.font_family'];
  const theme = settings['terminal.default_theme'];
  return {
    fontFamily: typeof family === 'string' && family.trim() !== '' ? family : DEFAULT_FONT_FAMILY,
    fontSize: clamp(numberSetting(settings, 'terminal.font_size', 14), FONT_SIZE_MIN, FONT_SIZE_MAX),
    lineHeight: clamp(numberSetting(settings, 'terminal.line_height', 1.2), LINE_HEIGHT_MIN, LINE_HEIGHT_MAX),
    cursorStyle: enumSetting(settings, 'terminal.cursor_style', ['block', 'bar', 'underline'] as const, 'block'),
    cursorBlink: boolSetting(settings, 'terminal.cursor_blink', false),
    boldIsBright: boolSetting(settings, 'terminal.bold_is_bright', true),
    copyOnSelect: boolSetting(settings, 'terminal.copy_on_select', false),
    bellSound: boolSetting(settings, 'terminal.bell_sound', false),
    confirmMultilinePaste: boolSetting(settings, 'terminal.confirm_multiline_paste', true),
    autoreconnect: boolSetting(settings, 'terminal.autoreconnect', true),
    defaultTheme:
      typeof theme === 'object' && theme !== null && !Array.isArray(theme) && typeof theme.builtin === 'string'
        ? { builtin: theme.builtin }
        : { builtin: 'graphite' },
  };
}

export function useTerminalSettings(): TerminalSettings {
  const settings = useSettings();
  return useMemo(() => readTerminalSettings(settings), [settings]);
}
