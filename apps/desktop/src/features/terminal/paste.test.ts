// Paste filter (docs/security.md 12; UI 4.3): markers, ESC and control
// characters are removed, line breaks become CR, multi-line detection,
// bracketed paste wrapping.

import { describe, expect, it } from 'vitest';
import { filterPaste, isMultiline, pasteLineCount, pastePreview, wrapPaste } from './paste';

describe('paste', () => {
  it('removes bracketed paste markers, ESC and control characters but keeps TAB, CR and LF', () => {
    expect(filterPaste('echo \x1b[201~rm -rf /\x1b[200~')).toBe('echo rm -rf /');
    expect(filterPaste('a\x1bb\x07c\x00d\x7fe\x9bf')).toBe('abcdef');
    expect(filterPaste('a\tb')).toBe('a\tb');
    expect(filterPaste('one\r\ntwo\nthree\rfour')).toBe('one\rtwo\rthree\rfour');
  });

  it('counts lines and detects multi-line text', () => {
    expect(isMultiline(filterPaste('ls -la'))).toBe(false);
    expect(isMultiline(filterPaste('ls\n'))).toBe(true);
    expect(pasteLineCount(filterPaste('a\nb\nc'))).toBe(3);
    expect(pasteLineCount(filterPaste('a\nb\n'))).toBe(2);
    expect(pasteLineCount('')).toBe(0);
  });

  it('wraps the text only when the application enabled bracketed paste', () => {
    expect(wrapPaste('ls', false)).toBe('ls');
    expect(wrapPaste('ls\r', true)).toBe('\x1b[200~ls\r\x1b[201~');
  });

  it('previews lines and truncates long texts', () => {
    expect(pastePreview('a\rb\r')).toBe('a\nb');
    const many = Array.from({ length: 20 }, (_value, index) => `line ${index}`).join('\r');
    expect(pastePreview(many, 3)).toBe('line 0\nline 1\nline 2\n…');
  });
});
