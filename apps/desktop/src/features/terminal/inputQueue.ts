// Ordered input to a session. Each `session_write` / `session_inject` is a
// separate IPC call, and calls are not guaranteed to run in order, so input
// typed while a write is in flight waits and goes out with the next call.
// Consecutive typed chunks are merged; a chunk that carries a history
// candidate ends its batch (the candidate belongs to the write with that
// Enter); injected input is never merged with typed input (it is marked
// injected by the core, docs/security.md 10.1).
//
// Size: one call carries at most `MAX_CALL_BYTES` of UTF-8 (the host refuses
// more than 1 MiB). Merging stops before that size, and a larger item (a big
// paste) is split at code point boundaries; a history candidate rides on the
// last part only.
//
// Backpressure: while too much input waits for the terminal (the remote side
// stopped reading), the host answers `unavailable` without taking the input;
// the queue sends the same input again a little later, keeping the order.
//
// Owner: S6.

import { IpcError, sessionInject, sessionWrite, type HistoryCandidate, type Uuid } from '../../ipc';

interface Item {
  kind: 'write' | 'inject';
  data: string;
  /** UTF-8 length of `data`. */
  bytes: number;
  binary?: boolean;
  history?: HistoryCandidate;
}

export interface InputQueue {
  write(data: string, options?: { binary?: boolean; history?: HistoryCandidate }): void;
  inject(data: string): void;
  /** Resolves when everything queued so far was sent (tests). */
  idle(): Promise<void>;
}

/** Largest input of one IPC call, in bytes of UTF-8 (the host accepts 1 MiB). */
export const MAX_CALL_BYTES = 512 * 1024;
/** Wait before input refused for backpressure is sent again. */
export const RETRY_DELAY_MS = 50;

/** UTF-8 length of a string (binary input: one byte per code unit is at most this). */
export function utf8Length(text: string): number {
  let bytes = 0;
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code <= 0xdbff && index + 1 < text.length) {
      const next = text.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

/** Splits text into parts of at most `limit` bytes of UTF-8, never inside a surrogate pair. */
export function splitUtf8(text: string, limit = MAX_CALL_BYTES): string[] {
  if (text.length * 3 <= limit) {
    return [text];
  }
  const parts: string[] = [];
  let start = 0;
  let bytes = 0;
  let index = 0;
  while (index < text.length) {
    const code = text.charCodeAt(index);
    const next = index + 1 < text.length ? text.charCodeAt(index + 1) : 0;
    const pair = code >= 0xd800 && code <= 0xdbff && next >= 0xdc00 && next <= 0xdfff;
    const width = pair ? 2 : 1;
    const size = pair ? 4 : code < 0x80 ? 1 : code < 0x800 ? 2 : 3;
    if (bytes + size > limit && index > start) {
      parts.push(text.slice(start, index));
      start = index;
      bytes = 0;
    }
    bytes += size;
    index += width;
  }
  if (start < text.length) {
    parts.push(text.slice(start));
  }
  return parts;
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function createInputQueue(sessionId: Uuid): InputQueue {
  const items: Item[] = [];
  let running: Promise<void> | null = null;

  const takeBatch = (): Item | undefined => {
    const first = items.shift();
    if (first === undefined || first.kind === 'inject' || first.history !== undefined) {
      return first;
    }
    let data = first.data;
    let bytes = first.bytes;
    while (items.length > 0) {
      const next = items[0] as Item;
      if (
        next.kind !== 'write' ||
        (next.binary ?? false) !== (first.binary ?? false) ||
        bytes + next.bytes > MAX_CALL_BYTES
      ) {
        break;
      }
      items.shift();
      data += next.data;
      bytes += next.bytes;
      if (next.history !== undefined) {
        return { ...first, data, bytes, history: next.history };
      }
    }
    return { ...first, data, bytes };
  };

  const send = async (item: Item) => {
    for (;;) {
      try {
        if (item.kind === 'inject') {
          await sessionInject({ session_id: sessionId, data: item.data });
        } else {
          await sessionWrite({
            session_id: sessionId,
            data: item.data,
            ...(item.binary === true ? { binary: true } : {}),
            ...(item.history === undefined ? {} : { history: item.history }),
          });
        }
        return;
      } catch (reason) {
        if (reason instanceof IpcError && reason.code === 'unavailable') {
          // Too much input waits for the terminal: nothing was taken.
          await sleep(RETRY_DELAY_MS);
          continue;
        }
        // The session ended or is not connected: input is dropped, like the core does.
        return;
      }
    }
  };

  const pump = async () => {
    for (let item = takeBatch(); item !== undefined; item = takeBatch()) {
      await send(item);
    }
  };

  const kick = () => {
    if (running === null) {
      running = pump().finally(() => {
        running = null;
        if (items.length > 0) {
          kick();
        }
      });
    }
  };

  const push = (kind: Item['kind'], data: string, options: { binary?: boolean; history?: HistoryCandidate } = {}) => {
    const parts = splitUtf8(data);
    parts.forEach((part, index) => {
      const last = index === parts.length - 1;
      items.push({
        kind,
        data: part,
        bytes: utf8Length(part),
        binary: options.binary,
        history: last ? options.history : undefined,
      });
    });
    kick();
  };

  return {
    write(data, options = {}) {
      if (data === '') {
        return;
      }
      push('write', data, options);
    },
    inject(data) {
      if (data === '') {
        return;
      }
      push('inject', data);
    },
    async idle() {
      while (running !== null) {
        await running;
      }
    },
  };
}
