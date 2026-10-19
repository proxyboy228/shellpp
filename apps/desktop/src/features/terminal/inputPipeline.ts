// Input of one terminal view: typed keys, pastes, mouse reports and history
// inserts, in order, with the history capture (docs/security.md 10.1).
//
// The capture reads the terminal buffer, which shows what the server echoed.
// Input typed faster than the echo (fast typists, WebDriver, a slow link)
// would otherwise give a truncated command or a wrong prompt boundary, so:
//
// - at an Enter that ends a command line being recorded, the Enter waits
//   until the echo of the line arrived and output is quiet for a moment (at
//   most `maxMs`; with echo off nothing arrives and the core drops the line);
// - the first printable character after an Enter waits the same way for the
//   new prompt before the boundary is taken.
//
// Nothing waits when the session does not record history, in the alternate
// screen, or for pasted text (a multi-line paste is not delayed per line;
// its lines are captured as the buffer shows them, which fails closed).
//
// LF (Ctrl+J) ends the line like Enter does in most shells, but never
// carries a candidate: the next line takes a new boundary after the prompt.
//
// Owner: S6.

import type { EngineBuffer } from './engine/types';
import { hasPrintable, type HistoryCapture } from './historyCapture';
import type { InputQueue } from './inputQueue';

/** Output arrival, updated by the view's output channel. */
export interface OutputTracker {
  /** Number of output chunks received. */
  seq: number;
  /** `performance.now()` of the last chunk. */
  lastAt: number;
}

export interface InputTiming {
  /** Output must be quiet this long before the buffer is read. */
  quietMs: number;
  /** The longest wait for an echo or a prompt. */
  maxMs: number;
}

export const DEFAULT_TIMING: InputTiming = { quietMs: 30, maxMs: 250 };

export interface InputPipelineOptions {
  capture: HistoryCapture;
  buffer(): EngineBuffer;
  queue: InputQueue;
  output: OutputTracker;
  /** The session records history (`SessionInfo.record_history`). */
  record(): boolean;
  timing?: InputTiming;
}

export interface InputPipeline {
  /** Keys and IME input (`onData`). */
  typed(data: string): void;
  /** Filtered paste text (already wrapped for bracketed paste). */
  pasted(data: string): void;
  /** Mouse reports (`onBinary`). */
  binary(data: string): void;
  /** App-sent input (history insert): never recorded. */
  injected(data: string): void;
  /** Resolves when every input so far was handed to the queue (tests). */
  idle(): Promise<void>;
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function createInputPipeline(options: InputPipelineOptions): InputPipeline {
  const { capture, buffer, queue, output } = options;
  const timing = options.timing ?? DEFAULT_TIMING;
  let chain: Promise<void> = Promise.resolve();
  /** Output count when the last typed text was sent (its echo comes after). */
  let sentAt = output.seq;
  /** Output count when the last Enter was sent; null once the next line started. */
  let enterAt: number | null = null;

  const run = (step: () => void | Promise<void>) => {
    chain = chain.then(step).catch(() => undefined);
  };

  /** Waits until output arrived after `since` and is quiet, or `maxMs` passed. */
  const settle = async (since: number) => {
    const start = performance.now();
    for (;;) {
      const now = performance.now();
      if (now - start >= timing.maxMs || (output.seq > since && now - output.lastAt >= timing.quietMs)) {
        return;
      }
      await sleep(Math.min(10, timing.quietMs));
    }
  };

  const alternate = () => buffer().type === 'alternate';

  const writeText = (text: string) => {
    if (text === '') {
      return;
    }
    capture.process(text, buffer, options.record());
    sentAt = output.seq;
    queue.write(text);
  };

  const typed = async (data: string) => {
    let rest = data;
    while (rest !== '') {
      const index = rest.search(/[\r\n]/);
      const ending = index < 0 ? '' : rest[index];
      const text = index < 0 ? rest : rest.slice(0, index);
      rest = index < 0 ? '' : rest.slice(index + 1);
      if (text !== '') {
        if (enterAt !== null && !capture.hasBoundary() && hasPrintable(text) && options.record() && !alternate()) {
          // The first character of a new line: let the prompt arrive first.
          await settle(enterAt);
        }
        if (hasPrintable(text)) {
          enterAt = null;
        }
        writeText(text);
      }
      if (index < 0) {
        break;
      }
      if (ending === '\n') {
        // The shell may run the line: start over without a candidate.
        capture.process('\n', buffer, false);
        queue.write('\n');
        enterAt = output.seq;
        continue;
      }
      const recording = options.record() && capture.hasBoundary();
      if (recording && !capture.startedInAltScreen() && !alternate()) {
        // Let the echo of the line arrive before reading it.
        await settle(sentAt);
      }
      const candidate = capture.enter(buffer());
      queue.write('\r', recording && candidate !== null ? { history: candidate } : {});
      enterAt = output.seq;
    }
  };

  return {
    typed: (data) => run(() => typed(data)),
    pasted: (data) =>
      run(() => {
        for (const segment of capture.process(data, buffer, options.record())) {
          queue.write(segment.data, segment.history === undefined ? {} : { history: segment.history });
        }
        sentAt = output.seq;
        enterAt = data.includes('\r') ? output.seq : enterAt;
      }),
    binary: (data) => run(() => queue.write(data, { binary: true })),
    injected: (data) => run(() => queue.inject(data)),
    idle: async () => {
      let current: Promise<void> | null = null;
      while (current !== chain) {
        current = chain;
        await current;
      }
      await queue.idle();
    },
  };
}
