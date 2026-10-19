// Snapshot protocol: the event buffer and the typed error mapping.

import { afterEach, describe, expect, it } from 'vitest';
import { setBackend } from './backend';
import { IpcError, isIpcError, toIpcError } from './errors';
import { createEventBuffer, type BufferedEvent } from './events';
import { createMockBackend } from './mock';

afterEach(() => {
  setBackend(null);
});

const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe('createEventBuffer', () => {
  it('holds events until release and then forwards only newer ones', async () => {
    const backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
    setBackend(backend);
    const received: BufferedEvent[] = [];
    const buffer = await createEventBuffer((event) => received.push(event));

    backend.control.notify('notifications.first');
    backend.control.notify('notifications.second');
    await flush();
    expect(received).toEqual([]);

    // The snapshot already contains the first notification (v = 1).
    buffer.release(1);
    expect(received.map((event) => event.payload.v)).toEqual([2]);

    backend.control.notify('notifications.third');
    await flush();
    expect(received.map((event) => [event.name, event.payload.v])).toEqual([
      ['notifications:changed', 2],
      ['notifications:changed', 3],
    ]);

    buffer.unlisten();
    backend.control.notify('notifications.fourth');
    await flush();
    expect(received).toHaveLength(2);
  });
});

describe('toIpcError', () => {
  it('keeps known codes and details and maps anything else to internal', () => {
    const error = toIpcError({ code: 'validation_failed', details: { errors: [{ field: 'ssh.port', reason: 'out_of_range' }] } });
    expect(error).toBeInstanceOf(IpcError);
    expect(error.fieldReason('ssh.port')).toBe('out_of_range');
    expect(error.fieldReason('address')).toBeUndefined();
    expect(toIpcError({ code: 'something_else', details: {} }).code).toBe('internal');
    expect(toIpcError(new Error('boom')).code).toBe('internal');
    expect(toIpcError('text').code).toBe('internal');
    expect(isIpcError(error, 'validation_failed')).toBe(true);
    expect(isIpcError(error, 'locked')).toBe(false);
    expect(toIpcError(error)).toBe(error);
  });
});
