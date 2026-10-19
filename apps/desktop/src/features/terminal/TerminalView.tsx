// One terminal (xterm.js) per pane leaf (UI 4.1, 4.3; stage 2 plan, section
// 6.2). It stays mounted while its tab is inactive or the window is locked,
// so scrollback and live output survive.
//
// - Output: a channel passed to `session_attach` (replayed buffer first, then
//   live output, raw bytes) written into the engine. Flow control: the bytes
//   the engine has processed are acknowledged with `session_ack` every
//   `ACK_STEP_BYTES`; the host stops reading output while the terminal is
//   more than 2 MiB behind (docs/architecture.md, section 3.5).
// - Input: typed data goes through the history capture (candidates ride on
//   the write with the Enter, only when the session records history) and an
//   ordered input queue. Backspace sends `\x08` for hosts set to `ctrl_h`.
// - App shortcuts never reach the shell (`isAppShortcut`, D17).
// - Right click pastes; the browser paste event (Cmd+V on macOS) goes through
//   the same filter; multi-line paste asks first (UI 4.3, docs/security.md 12).
// - Copy on select when `terminal.copy_on_select` is on; Ctrl+Shift+C copies.
// - Links: Ctrl+click (Cmd+click on macOS), http and https only; OSC 8 links
//   are confirmed with their full target.
// - Size: fitted to the pane; `session_resize` 50 ms after the size settles,
//   only when columns or rows change.
// - e2e builds register a probe per pane (src/ipc/e2e.ts).
//
// Owner: S6.

import { useEffect, useRef, useState } from 'react';
import { showErrorToast } from '../../app/errors';
import { isMac } from '../../app/platform';
import { useOs } from '../../app/store';
import {
  clipboardReadText,
  clipboardWriteText,
  externalOpenUrl,
  getBackend,
  sessionAck,
  sessionAttach,
  sessionResize,
  type OsKind,
  type SessionInfo,
} from '../../ipc';
import { registerTerminalProbe, setActiveTerminalProbe } from '../../ipc/e2e';
import { activatePane, rememberTerminalSize } from './api';
import { playBell } from './bell';
import { registerPaneController, setSearchResults } from './controllers';
import { linkRequestStore, pasteRequestStore } from './dialogs';
import { createEngine } from './engine/factory';
import type { EngineOptions, LinkActivation, TerminalEngine } from './engine/types';
import { HistoryCapture } from './historyCapture';
import { createInputPipeline, type OutputTracker } from './inputPipeline';
import { createInputQueue } from './inputQueue';
import { filterPaste, isMultiline, pasteLineCount, wrapPaste } from './paste';
import { isAppShortcut } from './shortcuts';
import { useTerminalSettings, type TerminalSettings } from './terminalSettings';
import { useSessionTheme, type BuiltinTheme } from './themes';
import type { PaneId } from './workspace';
import styles from './TerminalHost.module.css';

const RESIZE_DEBOUNCE_MS = 50;
const COPY_ON_SELECT_DEBOUNCE_MS = 150;
/** Processed output acknowledged at once (well under the host's 512 KiB resume mark). */
export const ACK_STEP_BYTES = 128 * 1024;

interface LiveValues {
  session: SessionInfo;
  settings: TerminalSettings;
  os: OsKind | null;
  theme: BuiltinTheme;
}

function engineOptions(live: LiveValues): EngineOptions {
  return {
    fontFamily: live.settings.fontFamily,
    fontSize: live.settings.fontSize,
    lineHeight: live.settings.lineHeight,
    cursorStyle: live.settings.cursorStyle,
    cursorBlink: live.settings.cursorBlink,
    boldIsBright: live.settings.boldIsBright,
    theme: live.theme.theme,
  };
}

/** http and https only (docs/security.md 12). */
function isOpenableLink(uri: string): boolean {
  if (!URL.canParse(uri)) {
    return false;
  }
  const url = new URL(uri);
  return (url.protocol === 'http:' || url.protocol === 'https:') && uri.length <= 2048;
}

/** Ctrl+click, or Cmd+click on macOS. */
function hasLinkModifier(event: MouseEvent, os: OsKind | null): boolean {
  return isMac(os) ? event.metaKey : event.ctrlKey;
}

function handleLink(activation: LinkActivation, os: OsKind | null): void {
  if (!hasLinkModifier(activation.event, os) || !isOpenableLink(activation.uri)) {
    return;
  }
  if (activation.kind === 'osc8') {
    // The link text may differ from the target: confirm with the full target,
    // normalized as it will be opened (punycode host, percent-encoded path),
    // so lookalike Unicode names and reordering characters cannot hide it.
    linkRequestStore.set({ url: new URL(activation.uri).href });
    return;
  }
  externalOpenUrl(activation.uri, 'terminal').catch(showErrorToast);
}

interface MountArgs {
  engine: TerminalEngine;
  container: HTMLElement;
  paneId: PaneId;
  live: { readonly current: LiveValues };
  capture: HistoryCapture;
  onRenderer(renderer: 'webgl' | 'dom'): void;
}

interface MountedTerminal {
  /** Fits the terminal again soon (after a font change) and resizes the PTY when the size changed. */
  refit(): void;
  dispose(): void;
}

/** Wires an engine to its session. */
function mountTerminal({ engine, container, paneId, live, capture, onRenderer }: MountArgs): MountedTerminal {
  const sessionId = live.current.session.session_id;
  const queue = createInputQueue(sessionId);
  const disposables: { dispose(): void }[] = [];
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let disposed = false;

  engine.open(container);
  onRenderer(engine.renderer());
  disposables.push(engine.onRendererChange(onRenderer));
  let size = engine.fit();
  rememberTerminalSize(size.cols, size.rows);

  const tracker: OutputTracker = { seq: 0, lastAt: 0 };
  const input = createInputPipeline({
    capture,
    buffer: () => engine.buffer(),
    queue,
    output: tracker,
    record: () => live.current.session.record_history,
  });

  const pasteText = (text: string | null) => {
    const filtered = filterPaste(text ?? '');
    if (filtered === '') {
      return;
    }
    const send = () => {
      input.pasted(wrapPaste(filtered, engine.bracketedPasteMode()));
      engine.focus();
    };
    if (isMultiline(filtered) && live.current.settings.confirmMultilinePaste) {
      pasteRequestStore.set({ text: filtered, lines: pasteLineCount(filtered), confirm: send });
    } else {
      send();
    }
  };

  // Output, with flow control.
  let unacked = 0;
  const processed = (bytes: number) => {
    if (disposed) {
      return;
    }
    unacked += bytes;
    if (unacked >= ACK_STEP_BYTES) {
      const acked = unacked;
      unacked = 0;
      sessionAck({ session_id: sessionId, bytes: acked }).catch(() => undefined);
    }
  };
  const channel = getBackend().createChannel<ArrayBuffer>((message) => {
    if (!disposed) {
      tracker.seq += 1;
      tracker.lastAt = performance.now();
      const bytes = new Uint8Array(message);
      engine.write(bytes, () => processed(bytes.byteLength));
    }
  });
  sessionAttach({ session_id: sessionId, on_output: channel })
    .then(() => {
      if (!disposed) {
        sessionResize({ session_id: sessionId, cols: size.cols, rows: size.rows }).catch(() => undefined);
      }
    })
    .catch(() => undefined);

  // Input.
  disposables.push(engine.onData((data) => input.typed(data)));
  disposables.push(engine.onBinary((data) => input.binary(data)));
  engine.setKeyHandler((event) => {
    if (isAppShortcut(event, live.current.os)) {
      return false;
    }
    if (
      event.key === 'Backspace' &&
      live.current.session.backspace === 'ctrl_h' &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.metaKey
    ) {
      if (event.type === 'keydown') {
        event.preventDefault();
        input.typed('\x08');
      }
      return false;
    }
    return true;
  });

  // Copy on select (UI 4.3; not a sensitive clipboard value, docs/security.md 9.4).
  let copyTimer: ReturnType<typeof setTimeout> | null = null;
  disposables.push(
    engine.onSelectionChange(() => {
      if (!live.current.settings.copyOnSelect) {
        return;
      }
      if (copyTimer !== null) {
        clearTimeout(copyTimer);
        timers.delete(copyTimer);
      }
      copyTimer = setTimeout(() => {
        copyTimer = null;
        if (!disposed && engine.hasSelection()) {
          clipboardWriteText(engine.getSelection()).catch(() => undefined);
        }
      }, COPY_ON_SELECT_DEBOUNCE_MS);
      timers.add(copyTimer);
    }),
  );
  disposables.push(
    engine.onBell(() => {
      if (live.current.settings.bellSound) {
        playBell();
      }
    }),
  );
  disposables.push(engine.onSearchResults((results) => setSearchResults(paneId, results)));

  // Mouse and clipboard events of the terminal element.
  const onContextMenu = (event: MouseEvent) => {
    event.preventDefault();
    activatePane(paneId);
    clipboardReadText()
      .then((result) => pasteText(result.text))
      .catch(showErrorToast);
  };
  const onPaste = (event: ClipboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    pasteText(event.clipboardData?.getData('text/plain') ?? '');
  };
  const onFocusIn = () => {
    activatePane(paneId);
    if (import.meta.env.MODE === 'e2e') {
      setActiveTerminalProbe(paneId);
    }
  };
  container.addEventListener('contextmenu', onContextMenu);
  container.addEventListener('paste', onPaste, true);
  container.addEventListener('focusin', onFocusIn);

  // Size.
  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  const refit = () => {
    resizeTimer = null;
    if (disposed) {
      return;
    }
    const next = engine.fit();
    if (next.cols !== size.cols || next.rows !== size.rows) {
      size = next;
      rememberTerminalSize(next.cols, next.rows);
      sessionResize({ session_id: sessionId, cols: next.cols, rows: next.rows }).catch(() => undefined);
    }
  };
  const scheduleRefit = () => {
    if (disposed) {
      return;
    }
    if (resizeTimer !== null) {
      clearTimeout(resizeTimer);
      timers.delete(resizeTimer);
    }
    resizeTimer = setTimeout(refit, RESIZE_DEBOUNCE_MS);
    timers.add(resizeTimer);
  };
  const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleRefit);
  observer?.observe(container);

  const unregisterController = registerPaneController(paneId, {
    sessionId,
    focus: () => engine.focus(),
    copySelection: async () => {
      if (engine.hasSelection()) {
        await clipboardWriteText(engine.getSelection()).catch(showErrorToast);
      }
    },
    pasteFromClipboard: async () => {
      try {
        pasteText((await clipboardReadText()).text);
      } catch (reason) {
        showErrorToast(reason);
      }
    },
    selectAll: () => engine.selectAll(),
    insert: (text) => {
      // Inserted without Enter and marked as injected (never recorded).
      const filtered = filterPaste(text).replace(/\r/g, ' ');
      if (filtered !== '') {
        input.injected(filtered);
        engine.focus();
      }
    },
    findNext: (term) => void engine.findNext(term),
    findPrevious: (term) => void engine.findPrevious(term),
    clearSearch: () => {
      engine.clearSearch();
      setSearchResults(paneId, null);
    },
  });

  const unregisterProbe =
    import.meta.env.MODE === 'e2e'
      ? registerTerminalProbe(paneId, {
          sessionId: () => sessionId,
          text: () => engine.normalText(),
          selection: () => engine.getSelection(),
          renderer: () => engine.renderer(),
          size: () => ({ cols: engine.cols, rows: engine.rows }),
        })
      : () => undefined;

  const dispose = () => {
    disposed = true;
    for (const timer of timers) {
      clearTimeout(timer);
    }
    observer?.disconnect();
    container.removeEventListener('contextmenu', onContextMenu);
    container.removeEventListener('paste', onPaste, true);
    container.removeEventListener('focusin', onFocusIn);
    for (const disposable of disposables) {
      disposable.dispose();
    }
    channel.close();
    unregisterController();
    unregisterProbe();
    setSearchResults(paneId, null);
    engine.dispose();
  };
  return { refit: scheduleRefit, dispose };
}

export interface TerminalViewProps {
  session: SessionInfo;
  paneId: PaneId;
  /** The pane is the active pane of the visible, active tab: it takes keyboard focus. */
  focused: boolean;
}

export function TerminalView({ session, paneId, focused }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const mountedRef = useRef<MountedTerminal | null>(null);
  const [capture] = useState(() => new HistoryCapture());
  const settings = useTerminalSettings();
  const os = useOs();
  const theme = useSessionTheme(session.session_id, session.terminal_theme);
  const live = useRef<LiveValues>({ session, settings, os, theme });
  const [renderer, setRenderer] = useState<'webgl' | 'dom'>('dom');
  const [ready, setReady] = useState(false);
  const sessionId = session.session_id;

  useEffect(() => {
    live.current = { session, settings, os, theme };
  });

  useEffect(() => {
    const container = containerRef.current;
    if (container === null) {
      return undefined;
    }
    let cancelled = false;
    let mounted: MountedTerminal | null = null;
    createEngine({ ...engineOptions(live.current), onLink: (activation) => handleLink(activation, live.current.os) })
      .then((engine) => {
        if (cancelled) {
          engine.dispose();
          return;
        }
        engineRef.current = engine;
        mounted = mountTerminal({
          engine,
          container,
          paneId,
          live,
          capture,
          onRenderer: setRenderer,
        });
        mountedRef.current = mounted;
        setReady(true);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      engineRef.current = null;
      mountedRef.current = null;
      mounted?.dispose();
      setReady(false);
    };
  }, [sessionId, paneId, capture]);

  // Display options follow the settings and the session's theme; a new font
  // size, family or line height changes the cell size, so the terminal is
  // fitted again (and the PTY resized).
  useEffect(() => {
    engineRef.current?.setOptions(engineOptions({ session, settings, os, theme }));
    mountedRef.current?.refit();
  }, [ready, session, settings, os, theme]);

  // A reconnect starts a new shell: forget the line being typed.
  const state = session.state;
  useEffect(() => {
    if (state === 'connected') {
      capture.reset();
    }
  }, [state, capture]);

  // Focus goes to the terminal when its tab or pane becomes active (UI 4.1).
  useEffect(() => {
    if (focused && ready) {
      engineRef.current?.focus();
      if (import.meta.env.MODE === 'e2e') {
        setActiveTerminalProbe(paneId);
      }
    }
  }, [focused, ready, paneId]);

  return (
    <div
      className={styles.view}
      style={{ background: theme.theme.background }}
      onMouseDown={(event) => {
        // A press on the padding focuses the terminal too.
        if (event.target === event.currentTarget) {
          event.preventDefault();
          engineRef.current?.focus();
        }
      }}
      data-testid="terminal-view"
      data-session-id={sessionId}
      data-renderer={renderer}
      data-theme={theme.slug}
    >
      <div ref={containerRef} className={styles.screen} />
    </div>
  );
}
