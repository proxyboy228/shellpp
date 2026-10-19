// The input pipeline waits for the echo before reading a command line and
// for the prompt before taking the next boundary, so fast typing (WebDriver,
// fast typists, slow links) records the right command (docs/security.md 10.1).

import { describe, expect, it } from 'vitest';
import type { HistoryCandidate } from '../../ipc';
import { FakeTerminalEngine } from './engine/fakeEngine';
import { HistoryCapture } from './historyCapture';
import { createInputPipeline, type OutputTracker } from './inputPipeline';
import type { InputQueue } from './inputQueue';
import { resolveTheme } from './themes';

const ECHO_DELAY_MS = 30;

function setup(options: { record?: boolean; echo?: boolean } = {}) {
  const engine = new FakeTerminalEngine({
    fontFamily: 'JetBrains Mono',
    fontSize: 14,
    lineHeight: 1.2,
    cursorStyle: 'block',
    cursorBlink: false,
    boldIsBright: true,
    theme: resolveTheme(null).theme,
    onLink: () => undefined,
  });
  const output: OutputTracker = { seq: 0, lastAt: 0 };
  const writes: { data: string; history?: HistoryCandidate; binary?: boolean; injected?: boolean }[] = [];
  let echoing = options.echo ?? true;
  const serverOutput = (text: string) => {
    output.seq += 1;
    output.lastAt = performance.now();
    engine.write(text);
  };
  // A server with a slow echo: input is echoed (and Enter answered with a new prompt) after a delay.
  const respond = (data: string) => {
    setTimeout(() => {
      let text = '';
      for (const char of data) {
        if (char === '\r') {
          text += '\r\n$ ';
          echoing = options.echo ?? true;
        } else if (echoing) {
          text += char;
        }
      }
      if (text !== '') {
        serverOutput(text);
      }
    }, ECHO_DELAY_MS);
  };
  const queue: InputQueue = {
    write: (data, extra = {}) => {
      writes.push({ data, ...extra });
      respond(data);
    },
    inject: (data) => {
      writes.push({ data, injected: true });
      respond(data);
    },
    idle: async () => undefined,
  };
  const pipeline = createInputPipeline({
    capture: new HistoryCapture(),
    buffer: () => engine.buffer(),
    queue,
    output,
    record: () => options.record ?? true,
    timing: { quietMs: 10, maxMs: 150 },
  });
  serverOutput('$ ');
  return { engine, pipeline, writes };
}

describe('input pipeline', () => {
  it('waits for the echo before reading a fast-typed command', async () => {
    const { pipeline, writes } = setup();
    pipeline.typed('drop\r');
    await pipeline.idle();
    expect(writes).toEqual([
      { data: 'drop' },
      { data: '\r', history: { command: 'drop', prompt_text: '$', typed_printable: true, alt_screen: false } },
    ]);
  });

  it('waits for the new prompt before taking the next boundary', async () => {
    const { pipeline, writes } = setup();
    pipeline.typed('echo one\r');
    pipeline.typed('echo two\r');
    await pipeline.idle();
    expect(writes.filter((write) => write.history !== undefined).map((write) => [write.history?.command, write.history?.prompt_text])).toEqual([
      ['echo one', '$'],
      ['echo two', '$'],
    ]);
  });

  it('gives up waiting when nothing is echoed (echo off): the empty line is dropped by the core', async () => {
    const { pipeline, writes } = setup({ echo: false });
    const start = performance.now();
    pipeline.typed('hunter2\r');
    await pipeline.idle();
    expect(performance.now() - start).toBeGreaterThanOrEqual(140);
    expect(writes[1]?.history).toMatchObject({ command: '', typed_printable: true });
  });

  it('does not wait when the session does not record history', async () => {
    const { pipeline, writes } = setup({ record: false });
    const start = performance.now();
    pipeline.typed('ls\r');
    pipeline.typed('pwd\r');
    await pipeline.idle();
    expect(performance.now() - start).toBeLessThan(ECHO_DELAY_MS);
    expect(writes).toEqual([{ data: 'ls' }, { data: '\r' }, { data: 'pwd' }, { data: '\r' }]);
  });

  it('ends the line at LF without a candidate and takes a new boundary after the prompt', async () => {
    const { pipeline, writes } = setup();
    pipeline.typed('a');
    pipeline.injected('X');
    pipeline.typed('\n');
    pipeline.typed('ls\r');
    await pipeline.idle();
    expect(writes.map((write) => write.data)).toEqual(['a', 'X', '\n', 'ls', '\r']);
    expect(writes[2]?.history).toBeUndefined();
    expect(writes[4]?.history?.command).toBe('ls');
  });

  it('keeps pastes, mouse reports and inserts in order without delays', async () => {
    const { pipeline, writes } = setup();
    pipeline.pasted('a\rb\r');
    pipeline.binary('\x1b[M !!');
    pipeline.injected('git status');
    await pipeline.idle();
    expect(writes.map((write) => write.data)).toEqual(['a\r', 'b\r', '\x1b[M !!', 'git status']);
    expect(writes[2]?.binary).toBe(true);
    expect(writes[3]?.injected).toBe(true);
  });
});
