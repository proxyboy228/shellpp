// The terminal engine interface: what the terminal view needs from xterm.js.
// The app uses the xterm.js engine (./xtermEngine.ts); Vitest uses the fake
// engine (./fakeEngine.ts), because jsdom has no canvas or WebGL (stage 2
// plan, section 6.2: "wrap xterm.js behind a small engine interface").
//
// Owner: S6.

import type { TerminalTheme } from '../../../theme/terminal-theme';

export interface Disposable {
  dispose(): void;
}

export type CursorStyle = 'block' | 'bar' | 'underline';

/** Display options; every field can change while the terminal runs. */
export interface EngineOptions {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  cursorStyle: CursorStyle;
  cursorBlink: boolean;
  /** `terminal.bold_is_bright`. */
  boldIsBright: boolean;
  theme: TerminalTheme;
}

/** A link activation: `plain` from the web-links addon, `osc8` from an OSC 8 hyperlink. */
export interface LinkActivation {
  kind: 'plain' | 'osc8';
  uri: string;
  event: MouseEvent;
}

export interface EngineCreateOptions extends EngineOptions {
  onLink(activation: LinkActivation): void;
}

/** One row of the active buffer. */
export interface EngineLine {
  /** Text with trailing spaces removed. */
  text: string;
  /** The row continues the previous one (soft wrap). */
  isWrapped: boolean;
  /** Text of the cells `start` (inclusive) to `end` (exclusive, default: the end of the row). */
  range(start: number, end?: number): string;
}

/**
 * A position in the normal buffer that follows the text when old rows are
 * trimmed from the scrollback (an xterm.js marker).
 */
export interface EngineMarker {
  /** The buffer row now; -1 once disposed. */
  readonly line: number;
  /** The row left the scrollback (or the marker was disposed). */
  readonly isDisposed: boolean;
  dispose(): void;
}

/** The active buffer as the history capture sees it (docs/security.md, section 10.1). */
export interface EngineBuffer {
  type: 'normal' | 'alternate';
  /** Cursor column in the viewport. */
  cursorX: number;
  /** Cursor row in the viewport. */
  cursorY: number;
  /** Buffer row of the top viewport row (scrollback length). */
  baseY: number;
  /** Rows in the buffer (scrollback and viewport). */
  length: number;
  /** A buffer row (absolute index), or undefined outside the buffer. */
  line(y: number): EngineLine | undefined;
  /** A marker on the cursor row of the normal buffer (undefined in the alternate buffer). */
  marker(): EngineMarker | undefined;
}

export interface SearchOptions {
  caseSensitive?: boolean;
}

export interface SearchResults {
  /** 0-based index of the active match, -1 when none. */
  index: number;
  count: number;
}

export interface TerminalEngine {
  /** Renders the terminal into the element (once). */
  open(container: HTMLElement): void;
  /** Writes output; `processed` runs once the terminal parsed it (flow control). */
  write(data: Uint8Array | string, processed?: () => void): void;
  focus(): void;
  /** Fits the terminal to its container; returns the new size. */
  fit(): { cols: number; rows: number };
  readonly cols: number;
  readonly rows: number;
  setOptions(options: EngineOptions): void;
  /** Typed input (keys, paste through the engine, IME). */
  onData(listener: (data: string) => void): Disposable;
  /** Binary input (some mouse reports): every UTF-16 code unit is one byte. */
  onBinary(listener: (data: string) => void): Disposable;
  onSelectionChange(listener: () => void): Disposable;
  onBell(listener: () => void): Disposable;
  /** Key events before the engine handles them; return false to let the app take the key. */
  setKeyHandler(handler: (event: KeyboardEvent) => boolean): void;
  hasSelection(): boolean;
  getSelection(): string;
  selectAll(): void;
  /** The application turned on bracketed paste mode (`ESC[?2004h`). */
  bracketedPasteMode(): boolean;
  buffer(): EngineBuffer;
  /** Scrollback and screen of the normal buffer, lines joined with `\n`, trailing spaces removed. */
  normalText(): string;
  findNext(term: string, options?: SearchOptions): boolean;
  findPrevious(term: string, options?: SearchOptions): boolean;
  clearSearch(): void;
  onSearchResults(listener: (results: SearchResults) => void): Disposable;
  renderer(): 'webgl' | 'dom';
  onRendererChange(listener: (renderer: 'webgl' | 'dom') => void): Disposable;
  dispose(): void;
}

export type EngineFactory = (options: EngineCreateOptions) => Promise<TerminalEngine>;
