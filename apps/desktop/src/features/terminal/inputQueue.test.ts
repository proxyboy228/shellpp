// The ordered input queue: merging, the per-call size limit (large pastes
// are split, never lost) and the retry while the host pushes back.

import { afterEach, describe, expect, it } from 'vitest';
import { IpcError, setBackend, type SessionWriteInput } from '../../ipc';
import { createRecordingBackend } from '../connect/testing/recordingBackend';
import { createInputQueue, MAX_CALL_BYTES, splitUtf8, utf8Length } from './inputQueue';

const SESSION_ID = '01990000-0000-7000-8000-0000000000d1';

afterEach(() => {
  setBackend(null);
});

describe('input queue', () => {
  it('measures and splits UTF-8 at code point boundaries', () => {
    expect(utf8Length('aé日\u{1f44d}')).toBe(1 + 2 + 3 + 4);
    const text = '\u{1f44d}'.repeat(10);
    const parts = splitUtf8(text, 9);
    expect(parts.join('')).toBe(text);
    for (const part of parts) {
      expect(utf8Length(part)).toBeLessThanOrEqual(9);
      // No part starts or ends inside a surrogate pair.
      expect(part.codePointAt(0)).toBe(0x1f44d);
    }
    expect(splitUtf8('short')).toEqual(['short']);
  });

  it('splits a paste larger than one call and keeps the candidate on the last part', async () => {
    const backend = createRecordingBackend();
    setBackend(backend);
    const queue = createInputQueue(SESSION_ID);
    const big = 'x'.repeat(MAX_CALL_BYTES * 2 + 10);
    const history = { command: 'x', prompt_text: '$', typed_printable: true, alt_screen: false };
    queue.write(big, { history });
    await queue.idle();
    const writes = backend.callsOf('session_write') as SessionWriteInput[];
    expect(writes).toHaveLength(3);
    expect(writes.map((write) => write.data).join('')).toBe(big);
    for (const write of writes) {
      expect(utf8Length(write.data)).toBeLessThanOrEqual(MAX_CALL_BYTES);
    }
    expect(writes.slice(0, 2).every((write) => write.history === undefined)).toBe(true);
    expect(writes[2]?.history).toEqual(history);
  });

  it('stops merging typed chunks before the size limit', async () => {
    const backend = createRecordingBackend();
    setBackend(backend);
    const queue = createInputQueue(SESSION_ID);
    const chunk = 'y'.repeat(MAX_CALL_BYTES / 2 + 1);
    queue.write(chunk);
    queue.write(chunk);
    queue.write(chunk);
    await queue.idle();
    const writes = backend.callsOf('session_write') as SessionWriteInput[];
    expect(writes.map((write) => write.data).join('')).toBe(chunk.repeat(3));
    for (const write of writes) {
      expect(utf8Length(write.data)).toBeLessThanOrEqual(MAX_CALL_BYTES);
    }
  });

  it('sends input again while the host pushes back, in order', async () => {
    let refusals = 2;
    const backend = createRecordingBackend({
      session_write: () => {
        if (refusals > 0) {
          refusals -= 1;
          throw new IpcError('unavailable');
        }
        return null;
      },
    });
    setBackend(backend);
    const queue = createInputQueue(SESSION_ID);
    queue.write('first');
    queue.inject('second');
    await queue.idle();
    const sent = backend.calls.map((call) => [call.command, (call.input as { data: string }).data]);
    expect(sent).toEqual([
      ['session_write', 'first'],
      ['session_write', 'first'],
      ['session_write', 'first'],
      ['session_inject', 'second'],
    ]);
  });

  it('drops input the host refuses for another reason', async () => {
    const backend = createRecordingBackend();
    backend.failNext('session_write', new IpcError('not_found'));
    setBackend(backend);
    const queue = createInputQueue(SESSION_ID);
    queue.write('lost');
    await queue.idle();
    queue.write('next');
    await queue.idle();
    expect((backend.callsOf('session_write') as SessionWriteInput[]).map((write) => write.data)).toEqual(['lost', 'next']);
  });
});
