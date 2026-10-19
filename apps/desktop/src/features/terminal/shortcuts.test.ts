// Shortcut capture rules (UI 12.5, D17): what the app takes while a terminal
// has focus, the terminal shortcuts, AltGr and the macOS set.

import { describe, expect, it } from 'vitest';
import { isAppShortcut, matchTerminalShortcut } from './shortcuts';

function key(key: string, init: KeyboardEventInit & { altGraph?: boolean } = {}): KeyboardEvent {
  const { altGraph = false, ...rest } = init;
  const event = new KeyboardEvent('keydown', { key, ...rest });
  Object.defineProperty(event, 'getModifierState', { value: (name: string) => name === 'AltGraph' && altGraph });
  return event;
}

describe('isAppShortcut (PC)', () => {
  it('takes Ctrl+1..9, Ctrl+, and every Ctrl+Shift combination', () => {
    expect(isAppShortcut(key('1', { ctrlKey: true }), 'windows')).toBe(true);
    expect(isAppShortcut(key('9', { ctrlKey: true }), 'linux')).toBe(true);
    expect(isAppShortcut(key(',', { ctrlKey: true }), 'windows')).toBe(true);
    expect(isAppShortcut(key('L', { ctrlKey: true, shiftKey: true }), 'windows')).toBe(true);
    expect(isAppShortcut(key('C', { ctrlKey: true, shiftKey: true }), 'windows')).toBe(true);
  });

  it('leaves Ctrl+letter without Shift to the shell', () => {
    for (const letter of ['l', 'k', 'c', 'p', 'd', 'r']) {
      expect(isAppShortcut(key(letter, { ctrlKey: true }), 'windows')).toBe(false);
    }
    expect(isAppShortcut(key('a'), 'windows')).toBe(false);
    expect(isAppShortcut(key('Enter'), 'windows')).toBe(false);
  });

  it('takes Ctrl+Alt+arrows and Alt+Left/Right, but nothing while AltGr is active', () => {
    expect(isAppShortcut(key('ArrowUp', { ctrlKey: true, altKey: true }), 'windows')).toBe(true);
    expect(isAppShortcut(key('ArrowLeft', { altKey: true }), 'windows')).toBe(true);
    expect(isAppShortcut(key('ArrowUp', { altKey: true }), 'windows')).toBe(false);
    expect(isAppShortcut(key('s', { ctrlKey: true, altKey: true }), 'windows')).toBe(false);
    expect(isAppShortcut(key('ArrowUp', { ctrlKey: true, altKey: true, altGraph: true }), 'windows')).toBe(false);
    expect(isAppShortcut(key('@', { ctrlKey: true, altKey: true, altGraph: true }), 'windows')).toBe(false);
  });
});

describe('isAppShortcut (macOS)', () => {
  it('gives every Cmd combination to the app and every Ctrl or Option combination to the shell', () => {
    expect(isAppShortcut(key('k', { metaKey: true }), 'macos')).toBe(true);
    expect(isAppShortcut(key('1', { metaKey: true }), 'macos')).toBe(true);
    expect(isAppShortcut(key('l', { ctrlKey: true }), 'macos')).toBe(false);
    expect(isAppShortcut(key('L', { ctrlKey: true, shiftKey: true }), 'macos')).toBe(false);
    expect(isAppShortcut(key('ArrowLeft', { altKey: true }), 'macos')).toBe(false);
  });
});

describe('matchTerminalShortcut', () => {
  it('maps Ctrl+Shift letters by key code', () => {
    const cases: [string, string][] = [
      ['KeyC', 'copy'],
      ['KeyV', 'paste'],
      ['KeyA', 'select-all'],
      ['KeyF', 'search'],
      ['KeyD', 'split-right'],
      ['KeyE', 'split-down'],
    ];
    for (const [code, shortcut] of cases) {
      const letter = code.slice(3);
      expect(matchTerminalShortcut(key(letter, { code, ctrlKey: true, shiftKey: true }), 'windows')).toBe(shortcut);
    }
    expect(matchTerminalShortcut(key('d', { code: 'KeyD', ctrlKey: true }), 'windows')).toBeNull();
    expect(matchTerminalShortcut(key('X', { code: 'KeyX', ctrlKey: true, shiftKey: true }), 'windows')).toBeNull();
  });

  it('maps pane focus to Ctrl+Alt+arrows (Cmd+Option on macOS)', () => {
    expect(matchTerminalShortcut(key('ArrowRight', { ctrlKey: true, altKey: true }), 'windows')).toBe('focus-right');
    expect(matchTerminalShortcut(key('ArrowDown', { metaKey: true, altKey: true }), 'macos')).toBe('focus-down');
    expect(matchTerminalShortcut(key('ArrowDown', { ctrlKey: true, altKey: true }), 'macos')).toBeNull();
    expect(matchTerminalShortcut(key('ArrowDown', { ctrlKey: true, altKey: true, altGraph: true }), 'windows')).toBeNull();
  });

  it('uses Cmd+Shift on macOS', () => {
    expect(matchTerminalShortcut(key('D', { code: 'KeyD', metaKey: true, shiftKey: true }), 'macos')).toBe('split-right');
    expect(matchTerminalShortcut(key('D', { code: 'KeyD', ctrlKey: true, shiftKey: true }), 'macos')).toBeNull();
  });
});
