// The xterm.js 6 engine (stage 2 plan, section 6.2; decision 8).
//
// - Addons: fit, search (match decorations in the theme's search colors),
//   unicode11 (`activeVersion = '11'`), web-links (http/https only, opened by
//   the view on Ctrl+click / Cmd+click) and WebGL. When WebGL is missing or
//   its context is lost, the WebGL addon is disposed and xterm.js keeps
//   rendering with its built-in DOM renderer (no canvas addon in xterm.js 6).
// - Security (docs/security.md, section 12): `windowOptions` stay at their
//   disabled defaults (no title reports, no window operations), titles are
//   never read (`onTitleChange` is not used), no clipboard (OSC 52) and no
//   image addon; OSC 8 links go through the view's `onLink` (confirmation),
//   non-HTTP schemes are never passed.
//
// Owner: S6.

import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon, type ISearchOptions } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal, type ITheme } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import type { TerminalTheme } from '../../../theme/terminal-theme';
import type {
  Disposable,
  EngineBuffer,
  EngineCreateOptions,
  EngineOptions,
  SearchOptions,
  SearchResults,
  TerminalEngine,
} from './types';

/** xterm.js theme roles from our theme format (docs/design-system.md, 7.2). */
export function toXtermTheme(theme: TerminalTheme): ITheme {
  return {
    background: theme.background,
    foreground: theme.foreground,
    cursor: theme.cursor,
    cursorAccent: theme.cursorText,
    selectionBackground: theme.selection,
    ...(theme.selectionForeground === undefined ? {} : { selectionForeground: theme.selectionForeground }),
    ...theme.ansi,
  };
}

/** The font stack: the chosen family, the bundled default, then any monospace font. */
export function fontStack(family: string): string {
  const clean = family.replace(/["\\]/g, '').trim();
  const families = [clean, 'JetBrains Mono'].filter((name, index, all) => name !== '' && all.indexOf(name) === index);
  return [...families.map((name) => `"${name}"`), 'monospace'].join(', ');
}

function terminalOptions(options: EngineOptions) {
  return {
    fontFamily: fontStack(options.fontFamily),
    fontSize: options.fontSize,
    lineHeight: options.lineHeight,
    cursorStyle: options.cursorStyle,
    cursorBlink: options.cursorBlink,
    drawBoldTextInBrightColors: options.boldIsBright,
    theme: toXtermTheme(options.theme),
  };
}

class XtermEngine implements TerminalEngine {
  private readonly term: Terminal;
  private readonly fitAddon = new FitAddon();
  private readonly searchAddon = new SearchAddon({ highlightLimit: 1000 });
  private webgl: WebglAddon | null = null;
  private currentRenderer: 'webgl' | 'dom' = 'dom';
  private readonly rendererListeners = new Set<(renderer: 'webgl' | 'dom') => void>();
  private theme: TerminalTheme;
  private disposed = false;

  constructor(options: EngineCreateOptions) {
    this.theme = options.theme;
    this.term = new Terminal({
      ...terminalOptions(options),
      // Unicode 11 widths and search decorations are proposed APIs.
      allowProposedApi: true,
      scrollback: 10_000,
      rightClickSelectsWord: false,
      macOptionIsMeta: false,
      linkHandler: {
        allowNonHttpProtocols: false,
        activate: (event, uri) => options.onLink({ kind: 'osc8', uri, event }),
      },
    });
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(this.searchAddon);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.loadAddon(new WebLinksAddon((event, uri) => options.onLink({ kind: 'plain', uri, event })));
  }

  open(container: HTMLElement): void {
    this.term.open(container);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => this.dropWebgl());
      this.term.loadAddon(webgl);
      this.webgl = webgl;
      this.setRenderer('webgl');
    } catch {
      // No WebGL (GPU-less machines, some CI VMs): the DOM renderer stays.
      this.setRenderer('dom');
    }
  }

  private dropWebgl(): void {
    const webgl = this.webgl;
    this.webgl = null;
    try {
      webgl?.dispose();
    } catch {
      // Already gone with its context.
    }
    this.setRenderer('dom');
  }

  private setRenderer(renderer: 'webgl' | 'dom'): void {
    if (this.currentRenderer === renderer) {
      return;
    }
    this.currentRenderer = renderer;
    for (const listener of [...this.rendererListeners]) {
      listener(renderer);
    }
  }

  write(data: Uint8Array | string, processed?: () => void): void {
    if (!this.disposed) {
      this.term.write(data, processed);
    }
  }

  focus(): void {
    this.term.focus();
  }

  fit(): { cols: number; rows: number } {
    try {
      if (this.fitAddon.proposeDimensions() !== undefined) {
        this.fitAddon.fit();
      }
    } catch {
      // Not measurable yet (hidden or detached container).
    }
    return { cols: this.term.cols, rows: this.term.rows };
  }

  get cols(): number {
    return this.term.cols;
  }

  get rows(): number {
    return this.term.rows;
  }

  setOptions(options: EngineOptions): void {
    this.theme = options.theme;
    Object.assign(this.term.options, terminalOptions(options));
  }

  onData(listener: (data: string) => void): Disposable {
    return this.term.onData(listener);
  }

  onBinary(listener: (data: string) => void): Disposable {
    return this.term.onBinary(listener);
  }

  onSelectionChange(listener: () => void): Disposable {
    return this.term.onSelectionChange(listener);
  }

  onBell(listener: () => void): Disposable {
    return this.term.onBell(listener);
  }

  setKeyHandler(handler: (event: KeyboardEvent) => boolean): void {
    this.term.attachCustomKeyEventHandler(handler);
  }

  hasSelection(): boolean {
    return this.term.hasSelection();
  }

  getSelection(): string {
    return this.term.getSelection();
  }

  selectAll(): void {
    this.term.selectAll();
  }

  bracketedPasteMode(): boolean {
    return this.term.modes.bracketedPasteMode;
  }

  buffer(): EngineBuffer {
    const active = this.term.buffer.active;
    const term = this.term;
    return {
      type: active.type,
      cursorX: active.cursorX,
      cursorY: active.cursorY,
      baseY: active.baseY,
      length: active.length,
      line: (y) => {
        const line = active.getLine(y);
        return line === undefined
          ? undefined
          : {
              text: line.translateToString(true),
              isWrapped: line.isWrapped,
              range: (start, end) => line.translateToString(false, start, end),
            };
      },
      marker: () => (active.type === 'normal' ? term.registerMarker(0) : undefined),
    };
  }

  normalText(): string {
    const normal = this.term.buffer.normal;
    const lines: string[] = [];
    for (let y = 0; y < normal.length; y += 1) {
      lines.push(normal.getLine(y)?.translateToString(true) ?? '');
    }
    while (lines.length > 0 && lines[lines.length - 1] === '') {
      lines.pop();
    }
    return lines.join('\n');
  }

  private searchOptions(options: SearchOptions | undefined): ISearchOptions {
    return {
      caseSensitive: options?.caseSensitive ?? false,
      decorations: {
        matchBackground: this.theme.searchMatch,
        matchOverviewRuler: this.theme.searchMatch,
        activeMatchBackground: this.theme.searchMatchActive,
        activeMatchColorOverviewRuler: this.theme.searchMatchActive,
      },
    };
  }

  findNext(term: string, options?: SearchOptions): boolean {
    return this.searchAddon.findNext(term, this.searchOptions(options));
  }

  findPrevious(term: string, options?: SearchOptions): boolean {
    return this.searchAddon.findPrevious(term, this.searchOptions(options));
  }

  clearSearch(): void {
    this.searchAddon.clearDecorations();
    this.term.clearSelection();
  }

  onSearchResults(listener: (results: SearchResults) => void): Disposable {
    return this.searchAddon.onDidChangeResults((event) =>
      listener({ index: event.resultIndex, count: event.resultCount }),
    );
  }

  renderer(): 'webgl' | 'dom' {
    return this.currentRenderer;
  }

  onRendererChange(listener: (renderer: 'webgl' | 'dom') => void): Disposable {
    this.rendererListeners.add(listener);
    return { dispose: () => this.rendererListeners.delete(listener) };
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.rendererListeners.clear();
    this.term.dispose();
  }
}

export async function createXtermEngine(options: EngineCreateOptions): Promise<TerminalEngine> {
  return new XtermEngine(options);
}
