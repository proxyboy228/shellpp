// History capture boundary logic (docs/security.md 10.1 rules 1-3) with the
// fake engine: the server echo is written to the engine as a real server
// would, the typed input goes through `HistoryCapture.process`.

import { describe, expect, it } from 'vitest';
import { FakeTerminalEngine } from './engine/fakeEngine';
import type { EngineCreateOptions } from './engine/types';
import { HistoryCapture } from './historyCapture';
import { resolveTheme } from './themes';

const OPTIONS: EngineCreateOptions = {
  fontFamily: 'JetBrains Mono',
  fontSize: 14,
  lineHeight: 1.2,
  cursorStyle: 'block',
  cursorBlink: false,
  boldIsBright: true,
  theme: resolveTheme(null).theme,
  onLink: () => undefined,
};

function setup(cols = 80) {
  const engine = new FakeTerminalEngine(OPTIONS);
  engine.cols = cols;
  const capture = new HistoryCapture();
  /** Types like a user: the capture sees the input first, then the server echoes it. */
  const type = (data: string, record = true) => {
    const segments = capture.process(data, () => engine.buffer(), record);
    for (const char of data) {
      if (char === '\r') {
        engine.write('\r\n');
      } else if (char >= ' ') {
        engine.write(char);
      }
    }
    return segments;
  };
  return { engine, capture, type };
}

describe('HistoryCapture', () => {
  it('takes the command from the boundary to the end of the line, without the prompt', () => {
    const { engine, type } = setup();
    engine.write('Welcome\r\nalice@host:~$ ');
    for (const char of 'ls -la') {
      expect(type(char)).toEqual([{ data: char }]);
    }
    const [segment] = type('\r');
    expect(segment).toEqual({
      data: '\r',
      history: { command: 'ls -la', prompt_text: 'alice@host:~$', typed_printable: true, alt_screen: false },
    });
  });

  it('joins wrapped rows and removes trailing spaces', () => {
    const { engine, type } = setup(20);
    engine.write('$ ');
    type('echo 0123456789abcdefghij   ');
    const [segment] = type('\r');
    expect(segment?.history?.command).toBe('echo 0123456789abcdefghij');
  });

  it('uses the last non-empty line above when the prompt row is empty before the boundary', () => {
    const { engine, type } = setup();
    engine.write('[sudo] password for test:\r\n');
    type('x');
    const [segment] = type('\r');
    expect(segment?.history?.prompt_text).toBe('[sudo] password for test:');
  });

  it('marks the alternate screen at the boundary or at Enter', () => {
    const { engine, type } = setup();
    engine.write('$ \x1b[?1049h');
    type('q');
    expect(type('\r')[0]?.history?.alt_screen).toBe(true);

    const second = setup();
    second.engine.write('$ ');
    second.type('vim');
    second.engine.write('\x1b[?1049h');
    expect(second.type('\r')[0]?.history?.alt_screen).toBe(true);
  });

  it('starts a new line after keys typed in the alternate screen without Enter', () => {
    const { engine, type } = setup();
    engine.write('$ less file\r\n\x1b[?1049h');
    // `q` quits the pager: no Enter, the normal screen comes back.
    type('q');
    engine.write('\x1b[?1049l$ ');
    type('ls');
    expect(type('\r')[0]?.history).toEqual({
      command: 'ls',
      prompt_text: '$',
      typed_printable: true,
      alt_screen: false,
    });
  });

  it('reports typed input whose echo is off (empty command, typed_printable)', () => {
    const { engine, capture } = setup();
    engine.write('Password: ');
    capture.process('hunter', () => engine.buffer(), true);
    const [segment] = capture.process('\r', () => engine.buffer(), true);
    expect(segment?.history).toMatchObject({ command: '', typed_printable: true });
  });

  it('sends no candidate for an empty line, after Ctrl+C, or when the session does not record', () => {
    const { engine, type } = setup();
    engine.write('$ ');
    expect(type('\r')).toEqual([{ data: '\r' }]);
    type('abc');
    type('\x03');
    expect(type('\r')).toEqual([{ data: '\r' }]);
    type('ls');
    expect(type('\r', false)).toEqual([{ data: '\r' }]);
  });

  it('skips escape sequences: arrows do not start a command', () => {
    const { engine, type } = setup();
    engine.write('$ ');
    expect(type('\x1b[A')).toEqual([{ data: '\x1b[A' }]);
    expect(type('\r')).toEqual([{ data: '\r' }]);
  });

  it('drops the boundary at LF (Ctrl+J): an inserted text never reaches a later candidate', () => {
    const { engine, capture, type } = setup();
    engine.write('$ ');
    type('a');
    // A history insert (injected input, echoed by the server).
    engine.write('X');
    // Ctrl+J: the shell runs `aX` and prints a new prompt.
    expect(capture.process('\n', () => engine.buffer(), true)).toEqual([{ data: '\n' }]);
    engine.write('\r\n$ ');
    type('ls');
    expect(type('\r')[0]?.history).toEqual({
      command: 'ls',
      prompt_text: '$',
      typed_printable: true,
      alt_screen: false,
    });
  });

  it('follows the boundary row when the full scrollback trims old rows', () => {
    const { engine, type } = setup(20);
    engine.rows = 3;
    engine.scrollback = 2;
    engine.write('one\r\ntwo\r\nthree\r\nfour\r\n$ ');
    // A long command wraps at the bottom: the top rows leave the scrollback.
    type('echo 0123456789abcdefghijklmnopqrstuvwxyz');
    const [segment] = type('\r');
    expect(segment?.history).toEqual({
      command: 'echo 0123456789abcdefghijklmnopqrstuvwxyz',
      prompt_text: '$',
      typed_printable: true,
      alt_screen: false,
    });
  });

  it('sends no candidate when the boundary row left the scrollback (fail-closed)', () => {
    const { engine, type } = setup(20);
    engine.rows = 3;
    engine.scrollback = 2;
    engine.write('$ ');
    type('export TOKEN=abc');
    // Output while typing pushes the boundary row out of the buffer.
    engine.write('\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n');
    type('def');
    expect(type('\r')).toEqual([{ data: '\r' }]);
  });

  it('splits input with several Enters into one segment per line', () => {
    const { engine, capture } = setup();
    engine.write('$ ');
    const segments = capture.process('a\rb\rc', () => engine.buffer(), true);
    expect(segments.map((segment) => segment.data)).toEqual(['a\r', 'b\r', 'c']);
    expect(segments[0]?.history).toBeDefined();
    expect(segments[1]?.history).toBeDefined();
    expect(segments[2]?.history).toBeUndefined();
  });
});
