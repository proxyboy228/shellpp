// History capture on the client (docs/security.md 10.1 rules 1-3; UI 4.2;
// D16; stage 2 plan, section 6.2). The webview only finds the candidate: the
// prompt boundary, the logical line at Enter, the prompt text and whether
// the alternate screen was active. The core applies the remaining rules
// (echo off, password prompt pattern, secret detector, injected input) and
// stores the command or drops it.
//
// - Boundary: the cursor position (buffer row including scrollback, column)
//   when the first printable character after an Enter (or the session start)
//   is typed. Paste counts as typed input. The row is held by a buffer marker,
//   so it stays right when old rows leave the full scrollback; a boundary
//   whose row left the scrollback gives no candidate (fail-closed).
// - At Enter (`\r` in the input): the candidate is the logical line from the
//   boundary (wrapped rows joined), trailing spaces removed; `prompt_text` is
//   the text before the boundary on its row, or the last non-empty line above
//   when that is empty; `alt_screen` when the alternate screen was active at
//   the boundary or at Enter.
// - Ctrl+C and Ctrl+D drop the boundary (the shell starts a new line); so
//   does LF (Ctrl+J), which also runs the line in most shells: only a real
//   Enter sends a candidate, the next line takes a new boundary.
//
// Owner: S6.

import type { HistoryCandidate } from '../../ipc';
import type { EngineBuffer, EngineMarker } from './engine/types';

/** A piece of input to send, with the history candidate when it ends with the Enter of a typed line. */
export interface InputSegment {
  data: string;
  history?: HistoryCandidate;
}

interface Boundary {
  row: number;
  col: number;
  altAtStart: boolean;
  /** Follows the row when the scrollback trims (normal buffer only). */
  marker: EngineMarker | undefined;
}

/** Length of the escape sequence at `index` (0 when `text[index]` is not ESC). */
function escapeLength(text: string, index: number): number {
  if (text[index] !== '\x1b') {
    return 0;
  }
  const next = text[index + 1];
  if (next === '[') {
    let end = index + 2;
    while (end < text.length) {
      const code = text.charCodeAt(end);
      if (code >= 0x40 && code <= 0x7e) {
        return end - index + 1;
      }
      end += 1;
    }
    return text.length - index;
  }
  if (next === 'O') {
    return Math.min(3, text.length - index);
  }
  return next === undefined ? 1 : 2;
}

function isPrintable(char: string): boolean {
  const code = char.codePointAt(0) ?? 0;
  return code >= 0x20 && code !== 0x7f && !(code >= 0x80 && code <= 0x9f);
}

/** True when the text contains a printable character outside escape sequences. */
export function hasPrintable(text: string): boolean {
  let index = 0;
  while (index < text.length) {
    const escape = escapeLength(text, index);
    if (escape > 0) {
      index += escape;
      continue;
    }
    const char = String.fromCodePoint(text.codePointAt(index) ?? 0);
    if (isPrintable(char)) {
      return true;
    }
    index += char.length;
  }
  return false;
}

/** The logical line starting at `row`: the row from `col`, plus every following wrapped row. */
function logicalLineFrom(buffer: EngineBuffer, row: number, col: number): string {
  const first = buffer.line(row);
  if (first === undefined) {
    return '';
  }
  let text = first.range(col);
  for (let y = row + 1; y < buffer.length; y += 1) {
    const line = buffer.line(y);
    if (line === undefined || !line.isWrapped) {
      break;
    }
    text += line.text;
  }
  return text.replace(/\s+$/u, '');
}

function promptTextBefore(buffer: EngineBuffer, row: number, col: number): string {
  const sameRow = (buffer.line(row)?.range(0, col) ?? '').replace(/\s+$/u, '');
  if (sameRow.trim() !== '') {
    return sameRow;
  }
  for (let y = row - 1; y >= 0; y -= 1) {
    const text = buffer.line(y)?.text ?? '';
    if (text.trim() !== '') {
      return text;
    }
  }
  return '';
}

export class HistoryCapture {
  private boundary: Boundary | null = null;
  private typedPrintable = false;

  /** Forgets the current line (a reconnect starts a new shell). */
  reset(): void {
    this.boundary?.marker?.dispose();
    this.boundary = null;
    this.typedPrintable = false;
  }

  /** A command line is being typed (the boundary is set). */
  hasBoundary(): boolean {
    return this.boundary !== null;
  }

  /** The line started in the alternate screen (its candidate is never stored). */
  startedInAltScreen(): boolean {
    return this.boundary?.altAtStart ?? false;
  }

  /**
   * Keys typed in a full-screen program without an Enter (such as `q` that
   * quits a pager) do not start a shell command line: once the normal screen
   * is back, the next printable character takes a new boundary.
   */
  private leftAltScreen(buffer: () => EngineBuffer): boolean {
    return this.boundary !== null && this.boundary.altAtStart && buffer().type === 'normal';
  }

  /**
   * Splits typed input into segments: one ending with each Enter, carrying
   * the candidate when `record` is true and a boundary exists.
   */
  process(data: string, buffer: () => EngineBuffer, record: boolean): InputSegment[] {
    const segments: InputSegment[] = [];
    let start = 0;
    let index = 0;
    while (index < data.length) {
      const escape = escapeLength(data, index);
      if (escape > 0) {
        index += escape;
        continue;
      }
      const char = String.fromCodePoint(data.codePointAt(index) ?? 0);
      if (char === '\r') {
        const segment: InputSegment = { data: data.slice(start, index + 1) };
        const candidate = this.enter(buffer());
        if (record && candidate !== null) {
          segment.history = candidate;
        }
        segments.push(segment);
        start = index + 1;
      } else if (char === '\x03' || char === '\x04' || char === '\n') {
        this.reset();
      } else if (isPrintable(char) && (this.boundary === null || this.leftAltScreen(buffer))) {
        const current = buffer();
        this.boundary?.marker?.dispose();
        this.boundary = {
          row: current.baseY + current.cursorY,
          col: current.cursorX,
          altAtStart: current.type === 'alternate',
          marker: current.type === 'normal' ? current.marker() : undefined,
        };
        this.typedPrintable = true;
      } else if (isPrintable(char)) {
        this.typedPrintable = true;
      }
      index += char.length;
    }
    if (start < data.length) {
      segments.push({ data: data.slice(start) });
    }
    return segments;
  }

  /** The candidate of the current line at Enter (null without a boundary); starts a new line. */
  enter(buffer: EngineBuffer): HistoryCandidate | null {
    const boundary = this.boundary;
    const typedPrintable = this.typedPrintable;
    let row = boundary?.row ?? 0;
    let lost = false;
    if (boundary?.marker !== undefined) {
      // The scrollback may have trimmed rows since: the marker knows the row.
      lost = boundary.marker.isDisposed || boundary.marker.line < 0;
      row = boundary.marker.line;
    }
    this.reset();
    if (boundary === null || lost) {
      return null;
    }
    return {
      command: logicalLineFrom(buffer, row, boundary.col),
      prompt_text: promptTextBefore(buffer, row, boundary.col),
      typed_printable: typedPrintable,
      alt_screen: boundary.altAtStart || buffer.type === 'alternate',
    };
  }
}
