// A fake terminal engine for Vitest (jsdom has no canvas or WebGL; stage 2
// plan, section 6.2). It keeps a tiny screen model: printable text, CR, LF,
// backspace, soft wraps at the column count, the alternate screen
// (`ESC[?1049h`/`l`) and bracketed paste mode (`ESC[?2004h`/`l`); other
// escape sequences are skipped. `scrollback` limits the normal buffer like
// xterm.js does (the oldest rows are trimmed and markers follow or are
// disposed). Tests drive it through `type`, `select` and `bell`, and read it
// through the `TerminalEngine` methods.
//
// Used automatically when `import.meta.env.MODE === 'test'` (./factory.ts).
//
// Owner: S6.

/* eslint-disable no-control-regex -- the screen model parses terminal escape sequences */

import type {
  Disposable,
  EngineBuffer,
  EngineCreateOptions,
  EngineLine,
  EngineMarker,
  EngineOptions,
  SearchOptions,
  SearchResults,
  TerminalEngine,
} from './types';

type Listener<T> = (value: T) => void;

class Emitter<T> {
  private readonly listeners = new Set<Listener<T>>();
  on(listener: Listener<T>): Disposable {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }
  fire(value: T): void {
    for (const listener of [...this.listeners]) {
      listener(value);
    }
  }
  clear(): void {
    this.listeners.clear();
  }
}

interface ScreenLine {
  chars: string[];
  isWrapped: boolean;
}

class FakeMarker implements EngineMarker {
  line: number;
  isDisposed = false;
  private readonly owner: Set<FakeMarker>;

  constructor(line: number, owner: Set<FakeMarker>) {
    this.line = line;
    this.owner = owner;
    owner.add(this);
  }

  dispose(): void {
    this.isDisposed = true;
    this.line = -1;
    this.owner.delete(this);
  }
}

class Screen {
  lines: ScreenLine[] = [{ chars: [], isWrapped: false }];
  row = 0;
  col = 0;
  readonly markers = new Set<FakeMarker>();

  private readonly size: () => { cols: number; rows: number; scrollback: number };

  constructor(size: () => { cols: number; rows: number; scrollback: number }) {
    this.size = size;
  }

  private ensureRow(row: number): void {
    while (this.lines.length <= row) {
      this.lines.push({ chars: [], isWrapped: false });
    }
    this.trim();
  }

  /** Drops the oldest rows beyond the scrollback; markers move or are disposed. */
  private trim(): void {
    const { rows, scrollback } = this.size();
    const excess = this.lines.length - (rows + scrollback);
    if (excess <= 0) {
      return;
    }
    this.lines.splice(0, excess);
    this.row -= excess;
    for (const marker of [...this.markers]) {
      marker.line -= excess;
      if (marker.line < 0) {
        marker.dispose();
      }
    }
  }

  print(char: string): void {
    const { cols } = this.size();
    if (this.col >= cols) {
      this.row += 1;
      this.ensureRow(this.row);
      (this.lines[this.row] as ScreenLine).isWrapped = true;
      this.col = 0;
    }
    this.ensureRow(this.row);
    const line = this.lines[this.row] as ScreenLine;
    while (line.chars.length < this.col) {
      line.chars.push(' ');
    }
    line.chars[this.col] = char;
    this.col += 1;
  }

  carriageReturn(): void {
    this.col = 0;
  }

  lineFeed(): void {
    this.row += 1;
    this.ensureRow(this.row);
  }

  backspace(): void {
    this.col = Math.max(0, this.col - 1);
  }

  eraseToEnd(): void {
    const line = this.lines[this.row];
    if (line !== undefined) {
      line.chars.length = Math.min(line.chars.length, this.col);
    }
  }

  text(row: number): string | undefined {
    const line = this.lines[row];
    return line === undefined ? undefined : line.chars.join('').replace(/\s+$/u, '');
  }
}

export class FakeTerminalEngine implements TerminalEngine {
  options: EngineCreateOptions;
  container: HTMLElement | null = null;
  disposed = false;
  focused = false;
  cols = 80;
  rows = 24;
  /** Everything written to the engine, decoded. */
  written = '';
  /** Size returned by the next `fit()` calls. */
  fitSize: { cols: number; rows: number } = { cols: 100, rows: 30 };
  selection = '';
  bracketed = false;
  rendererKind: 'webgl' | 'dom' = 'webgl';
  /** Rows kept above the screen in the normal buffer (xterm.js `scrollback`). */
  scrollback = Number.POSITIVE_INFINITY;
  /** `write` callbacks run at once unless held (flow control tests). */
  holdProcessed = false;
  readonly heldCallbacks: (() => void)[] = [];
  searches: { direction: 'next' | 'previous'; term: string }[] = [];
  keyHandler: ((event: KeyboardEvent) => boolean) | null = null;

  private readonly normal = new Screen(() => this);
  private alternate: Screen | null = null;
  private readonly decoder = new TextDecoder();
  private pending = '';
  private readonly data = new Emitter<string>();
  private readonly binary = new Emitter<string>();
  private readonly selectionChange = new Emitter<void>();
  private readonly bellEmitter = new Emitter<void>();
  private readonly searchResults = new Emitter<SearchResults>();
  private readonly rendererChange = new Emitter<'webgl' | 'dom'>();

  constructor(options: EngineCreateOptions) {
    this.options = options;
  }

  private get screen(): Screen {
    return this.alternate ?? this.normal;
  }

  open(container: HTMLElement): void {
    this.container = container;
    const textarea = document.createElement('textarea');
    textarea.className = 'xterm-helper-textarea';
    textarea.setAttribute('aria-label', 'Terminal input');
    const root = document.createElement('div');
    root.className = 'xterm';
    root.appendChild(textarea);
    container.appendChild(root);
  }

  write(data: Uint8Array | string, processed?: () => void): void {
    const text = typeof data === 'string' ? data : this.decoder.decode(data, { stream: true });
    this.written += text;
    this.parse(this.pending + text);
    if (processed !== undefined) {
      if (this.holdProcessed) {
        this.heldCallbacks.push(processed);
      } else {
        processed();
      }
    }
  }

  /** Runs the held `write` callbacks (flow control tests). */
  releaseProcessed(): void {
    for (const callback of this.heldCallbacks.splice(0)) {
      callback();
    }
  }

  private parse(text: string): void {
    this.pending = '';
    let index = 0;
    while (index < text.length) {
      const char = text[index] as string;
      if (char === '\x1b') {
        const rest = text.slice(index);
        const csi = /^\x1b\[([0-9;?]*)([\x40-\x7e])/.exec(rest);
        if (csi !== null) {
          this.csi(csi[1] ?? '', csi[2] ?? '');
          index += csi[0].length;
          continue;
        }
        const osc = /^\x1b\][^\x07\x1b]*(\x07|\x1b\\)/.exec(rest);
        if (osc !== null) {
          index += osc[0].length;
          continue;
        }
        if (/^\x1b(\[[0-9;?]*|\][^\x07]*)?$/.test(rest)) {
          // An incomplete sequence: wait for the rest.
          this.pending = rest;
          return;
        }
        index += 2;
        continue;
      }
      const code = char.codePointAt(0) ?? 0;
      if (char === '\r') {
        this.screen.carriageReturn();
      } else if (char === '\n') {
        this.screen.lineFeed();
      } else if (char === '\b') {
        this.screen.backspace();
      } else if (char === '\x07') {
        this.bellEmitter.fire();
      } else if (code >= 0x20 && code !== 0x7f) {
        const full = String.fromCodePoint(code);
        this.screen.print(full);
        index += full.length;
        continue;
      }
      index += 1;
    }
  }

  private csi(params: string, final: string): void {
    if (final === 'h' || final === 'l') {
      const on = final === 'h';
      if (params === '?1049' || params === '?47' || params === '?1047') {
        this.alternate = on ? new Screen(() => this) : null;
      } else if (params === '?2004') {
        this.bracketed = on;
      }
    } else if (final === 'K') {
      this.screen.eraseToEnd();
    }
  }

  focus(): void {
    this.focused = true;
    const textarea = this.container?.querySelector('textarea');
    textarea?.focus();
  }

  fit(): { cols: number; rows: number } {
    this.cols = this.fitSize.cols;
    this.rows = this.fitSize.rows;
    return { cols: this.cols, rows: this.rows };
  }

  setOptions(options: EngineOptions): void {
    this.options = { ...this.options, ...options };
  }

  onData(listener: (data: string) => void): Disposable {
    return this.data.on(listener);
  }

  onBinary(listener: (data: string) => void): Disposable {
    return this.binary.on(listener);
  }

  onSelectionChange(listener: () => void): Disposable {
    return this.selectionChange.on(listener);
  }

  onBell(listener: () => void): Disposable {
    return this.bellEmitter.on(listener);
  }

  setKeyHandler(handler: (event: KeyboardEvent) => boolean): void {
    this.keyHandler = handler;
  }

  hasSelection(): boolean {
    return this.selection !== '';
  }

  getSelection(): string {
    return this.selection;
  }

  selectAll(): void {
    this.select(this.normalText());
  }

  bracketedPasteMode(): boolean {
    return this.bracketed;
  }

  buffer(): EngineBuffer {
    const screen = this.screen;
    const baseY = Math.max(0, screen.lines.length - this.rows);
    return {
      type: this.alternate === null ? 'normal' : 'alternate',
      cursorX: screen.col,
      cursorY: screen.row - baseY,
      baseY,
      length: Math.max(screen.lines.length, baseY + this.rows),
      line: (y: number): EngineLine | undefined => {
        if (y < 0 || y >= Math.max(screen.lines.length, baseY + this.rows)) {
          return undefined;
        }
        const line = screen.lines[y];
        return {
          text: screen.text(y) ?? '',
          isWrapped: line?.isWrapped ?? false,
          range: (start, end) => (line?.chars ?? []).slice(start, end).join(''),
        };
      },
      marker: () => (this.alternate === null ? new FakeMarker(screen.row, screen.markers) : undefined),
    };
  }

  normalText(): string {
    const lines = this.normal.lines.map((_line, index) => this.normal.text(index) ?? '');
    while (lines.length > 0 && lines[lines.length - 1] === '') {
      lines.pop();
    }
    return lines.join('\n');
  }

  findNext(term: string, options?: SearchOptions): boolean {
    void options;
    this.searches.push({ direction: 'next', term });
    return this.reportSearch(term);
  }

  findPrevious(term: string, options?: SearchOptions): boolean {
    void options;
    this.searches.push({ direction: 'previous', term });
    return this.reportSearch(term);
  }

  private reportSearch(term: string): boolean {
    const text = this.normalText().toLowerCase();
    const needle = term.toLowerCase();
    let count = 0;
    if (needle !== '') {
      for (let at = text.indexOf(needle); at >= 0; at = text.indexOf(needle, at + needle.length)) {
        count += 1;
      }
    }
    this.searchResults.fire({ index: count > 0 ? 0 : -1, count });
    return count > 0;
  }

  clearSearch(): void {
    this.searches.push({ direction: 'next', term: '' });
  }

  onSearchResults(listener: (results: SearchResults) => void): Disposable {
    return this.searchResults.on(listener);
  }

  renderer(): 'webgl' | 'dom' {
    return this.rendererKind;
  }

  onRendererChange(listener: (renderer: 'webgl' | 'dom') => void): Disposable {
    return this.rendererChange.on(listener);
  }

  dispose(): void {
    this.disposed = true;
    for (const emitter of [this.data, this.binary, this.selectionChange, this.bellEmitter]) {
      emitter.clear();
    }
    this.container?.replaceChildren();
  }

  // --- Test helpers ----------------------------------------------------------

  /** Input as if typed (keys or IME): fires `onData`. */
  type(data: string): void {
    this.data.fire(data);
  }

  /** Mouse report bytes: fires `onBinary`. */
  typeBinary(data: string): void {
    this.binary.fire(data);
  }

  /** Sets the selection text and fires `onSelectionChange`. */
  select(text: string): void {
    this.selection = text;
    this.selectionChange.fire();
  }

  /** Simulates a lost WebGL context. */
  loseWebgl(): void {
    this.rendererKind = 'dom';
    this.rendererChange.fire('dom');
  }
}

const created: FakeTerminalEngine[] = [];

/** Engines created so far (oldest first), for tests. */
export function fakeEngines(): readonly FakeTerminalEngine[] {
  return created;
}

/** Forgets the created engines (between tests). */
export function clearFakeEngines(): void {
  created.length = 0;
}

export async function createFakeEngine(options: EngineCreateOptions): Promise<TerminalEngine> {
  const engine = new FakeTerminalEngine(options);
  created.push(engine);
  return engine;
}
