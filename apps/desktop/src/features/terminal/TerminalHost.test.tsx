// The terminal area with the mock backend and the fake engine (UI 4.1, 4.3,
// 4.6, 4.7): opening sessions, output and input, history candidates, focus,
// paste and links, Backspace, end overlays, splits and pane focus, lock.

import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUnlocked, resetApp } from '../../test/app';
import { settingsStore } from '../../app/store';
import {
  activateTab,
  applySnapshotSessions,
  closePane,
  openLocalTerminal,
  openTerminalSidebar,
  restoreLastSession,
  splitPane,
  toggleTerminalSidebar,
} from './api';
import { installTerminalShortcuts } from './shortcuts';
import {
  engineFor,
  openHost,
  openLocal,
  withVault,
  savedHost,
  secret,
  sessionOf,
  typeLine,
  waitForState,
} from './testing/helpers';
import { resetWorkspace, sessionsStore, workspaceStore } from './workspace';

vi.mock('../hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
  vi.useRealTimers();
});

function pressed(engine: { keyHandler: ((event: KeyboardEvent) => boolean) | null }, init: KeyboardEventInit & { key: string }) {
  const event = new KeyboardEvent('keydown', { cancelable: true, ...init });
  return { allowed: engine.keyHandler?.(event) ?? true, event };
}

describe('TerminalHost', () => {
  it('opens a local terminal in a new active tab: output, focus, typed input and history', async () => {
    const { backend } = await withVault(renderUnlocked());
    const { tabId, sessionId, paneId } = await openLocal();
    expect(workspaceStore.get().active).toBe(tabId);
    expect(screen.getByTestId('terminal-host')).not.toHaveAttribute('hidden');
    expect(screen.getByTestId('terminal-host')).toHaveAttribute('data-active-tab', tabId);
    const pane = screen.getByTestId('terminal-pane');
    expect(pane).toHaveAttribute('data-pane-id', paneId);
    expect(pane).toHaveAttribute('data-session-id', sessionId);
    expect(pane).toHaveAttribute('data-state', 'connected');

    const engine = await engineFor(sessionId);
    const view = screen.getByTestId('terminal-view');
    expect(view).toHaveAttribute('data-theme', 'graphite');
    await waitFor(() => expect(view).toHaveAttribute('data-renderer', 'webgl'));
    await waitFor(() => expect(engine.written).toContain('PS C:\\mock> '));
    await waitFor(() => expect(engine.focused).toBe(true));
    expect(engine.options.fontFamily).toBe('JetBrains Mono');
    expect(engine.options.fontSize).toBe(14);

    await typeLine(engine, 'echo hi');
    await waitFor(() => expect(engine.written).toContain('echo hi\r\nhi\r\n'));
    expect(backend.control.sessions.input(sessionId)).toBe('echo hi\r');
    // The fitted size went to the session after attach.
    await typeLine(engine, 'size');
    await waitFor(() => expect(engine.written).toContain('100x30'));
    const history = await backend.invoke('history_list', {});
    expect(history.map((entry) => entry.command)).toEqual(['size', 'echo hi']);

    // The WebGL context can be lost: the DOM renderer takes over.
    act(() => engine.loseWebgl());
    await waitFor(() => expect(view).toHaveAttribute('data-renderer', 'dom'));

    // `exit` closes the tab (exit status 0) and the Vaults tab returns.
    await typeLine(engine, 'exit');
    await waitFor(() => expect(workspaceStore.get().tabs).toEqual([]));
    expect(workspaceStore.get().active).toBe('vaults');
    expect(screen.getByTestId('terminal-host')).toHaveAttribute('hidden');
    await waitFor(() => expect(engine.disposed).toBe(true));
  });

  it('shows the connection screen first, then the terminal of a saved host', async () => {
    const { backend, vaultId } = await withVault(renderUnlocked());
    const recordId = await savedHost(backend, vaultId, { label: 'db-1', terminal_theme: { builtin: 'paper' } }, false);
    const { sessionId } = await openHost(vaultId, recordId);
    // The password prompt keeps the session on the connection screen.
    await waitFor(() => expect(sessionOf(backend, sessionId)?.prompt?.kind).toBe('password'));
    await waitFor(() => expect(screen.getByTestId('connecting-view')).toHaveAttribute('data-session-id', sessionId));
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    const promptId = sessionOf(backend, sessionId)?.prompt?.prompt_id as number;
    await act(async () => {
      await backend.invoke('session_prompt_reply', { session_id: sessionId, prompt_id: promptId, reply: { kind: 'text', value: secret() } });
    });
    await waitForState(backend, sessionId, 'connected');
    const engine = await engineFor(sessionId);
    expect(screen.queryByTestId('connecting-view')).not.toBeInTheDocument();
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-theme', 'paper');
    expect(engine.options.theme.name).toBe('Paper');
    await waitFor(() => expect(engine.written).toContain('Welcome to shellpp-test-sshd\r\nalice@shellpp-test:~$ '));
    // The title bar tab shows the host label.
    const tab = screen.getByTestId('titlebar-session-tab');
    expect(within(tab).getByTestId('titlebar-session-tab-title')).toHaveTextContent('db-1');
    expect(tab).toHaveAttribute('data-state', 'connected');
  });

  it('shows "Session ended with code N" with Reconnect and Close', async () => {
    const { backend, vaultId, user } = await withVault(renderUnlocked());
    const recordId = await savedHost(backend, vaultId);
    const { sessionId } = await openHost(vaultId, recordId);
    await waitForState(backend, sessionId, 'connected');
    const engine = await engineFor(sessionId);
    await typeLine(engine, 'exit 3');
    const overlay = await screen.findByTestId('session-ended');
    expect(overlay).toHaveAttribute('data-exit-status', '3');
    expect(screen.getByTestId('session-ended-message')).toHaveTextContent('Session ended with code 3');
    // The output stays.
    expect(screen.getByTestId('terminal-view')).toBeInTheDocument();
    await user.click(screen.getByTestId('session-ended-reconnect'));
    await waitFor(() => expect(screen.queryByTestId('session-ended')).not.toBeInTheDocument());
    await waitForState(backend, sessionId, 'connected');
    await typeLine(engine, 'exit 4');
    await user.click(await screen.findByTestId('session-ended-close'));
    await waitFor(() => expect(workspaceStore.get().tabs).toEqual([]));
    expect(sessionOf(backend, sessionId)).toBeUndefined();
  });

  it('shows a drop with the reason, a countdown and Cancel; Close removes the tab', async () => {
    const { backend, vaultId, user } = await withVault(renderUnlocked());
    const recordId = await savedHost(backend, vaultId);
    const { sessionId } = await openHost(vaultId, recordId);
    await waitForState(backend, sessionId, 'connected');
    await engineFor(sessionId);
    backend.control.sessions.setRefusing(true);
    act(() => backend.control.sessions.drop(sessionId));
    const overlay = await screen.findByTestId('session-dropped');
    expect(overlay).toHaveAttribute('data-code', 'ssh_error');
    expect(overlay).toHaveAttribute('data-reason', 'disconnected');
    expect(screen.getByTestId('session-dropped-message')).toHaveTextContent('The connection was closed.');
    const countdown = screen.getByTestId('session-dropped-countdown');
    expect(Number(countdown.getAttribute('data-seconds'))).toBeGreaterThanOrEqual(0);
    expect(Number(countdown.getAttribute('data-seconds'))).toBeLessThanOrEqual(1);
    // The first attempt fails (refused): the next one is scheduled after 2 s.
    await waitFor(() => expect(screen.getByTestId('session-dropped')).toHaveAttribute('data-reason', 'refused'), { timeout: 3000 });
    await waitFor(() => expect(Number(screen.getByTestId('session-dropped-countdown').getAttribute('data-seconds'))).toBeGreaterThan(0));
    await user.click(screen.getByTestId('session-dropped-cancel'));
    await waitFor(() => expect(screen.queryByTestId('session-dropped-countdown')).not.toBeInTheDocument());
    expect(screen.queryByTestId('session-dropped-cancel')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('session-dropped-close'));
    await waitFor(() => expect(workspaceStore.get().tabs).toEqual([]));
  });

  it('pastes on right click: single lines directly, several lines after a confirmation, always filtered', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    const view = screen.getByTestId('terminal-view');

    backend.control.setClipboard('echo one');
    fireEvent.contextMenu(view.firstElementChild as Element);
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toBe('echo one'));

    backend.control.setClipboard('\recho two\x1b[201~\r\necho three\n');
    fireEvent.contextMenu(view.firstElementChild as Element);
    const dialog = await screen.findByTestId('paste-confirm-dialog');
    expect(within(dialog).getByTestId('paste-confirm-lines')).toHaveAttribute('data-count', '3');
    expect(within(dialog).getByTestId('paste-confirm-preview')).toHaveTextContent('echo two');
    await user.click(within(dialog).getByTestId('paste-confirm-cancel'));
    expect(backend.control.sessions.input(sessionId)).toBe('echo one');

    fireEvent.contextMenu(view.firstElementChild as Element);
    await user.click(await screen.findByTestId('paste-confirm-paste'));
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toBe('echo one\recho two\recho three\r'));

    // Bracketed paste mode wraps the text; the browser paste event uses the same filter.
    act(() => engine.write('\x1b[?2004h'));
    const paste = new Event('paste', { bubbles: true, cancelable: true }) as Event & { clipboardData: unknown };
    paste.clipboardData = { getData: () => 'ls\x07' };
    act(() => {
      view.firstElementChild?.dispatchEvent(paste);
    });
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toContain('\x1b[200~ls\x1b[201~'));
    expect(paste.defaultPrevented).toBe(true);
  });

  it('keeps app shortcuts away from the shell and sends ^H for hosts set to ctrl_h', async () => {
    const { backend, vaultId } = await withVault(renderUnlocked());
    const recordId = await savedHost(backend, vaultId, { backspace: 'ctrl_h' });
    const { sessionId } = await openHost(vaultId, recordId);
    await waitForState(backend, sessionId, 'connected');
    const engine = await engineFor(sessionId);
    expect(pressed(engine, { key: 'C', ctrlKey: true, shiftKey: true }).allowed).toBe(false);
    expect(pressed(engine, { key: '3', ctrlKey: true }).allowed).toBe(false);
    expect(pressed(engine, { key: 'l', ctrlKey: true }).allowed).toBe(true);
    expect(pressed(engine, { key: 'k', ctrlKey: true }).allowed).toBe(true);
    const backspace = pressed(engine, { key: 'Backspace' });
    expect(backspace.allowed).toBe(false);
    expect(backspace.event.defaultPrevented).toBe(true);
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toBe('\x08'));
  });

  it('opens plain links on Ctrl+click and confirms OSC 8 links with their target', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    const click = (init: MouseEventInit = {}) => new MouseEvent('click', init);
    act(() => engine.options.onLink({ kind: 'plain', uri: 'https://example.com/docs', event: click() }));
    act(() => engine.options.onLink({ kind: 'plain', uri: 'javascript:alert(1)', event: click({ ctrlKey: true }) }));
    act(() => engine.options.onLink({ kind: 'plain', uri: 'http://example.com/a', event: click({ ctrlKey: true }) }));
    await waitFor(() => expect(backend.control.openedUrls()).toEqual(['http://example.com/a']));

    act(() => engine.options.onLink({ kind: 'osc8', uri: 'https://example.org/target', event: click({ ctrlKey: true }) }));
    const dialog = await screen.findByTestId('link-confirm-dialog');
    expect(within(dialog).getByTestId('link-confirm-url')).toHaveTextContent('https://example.org/target');
    await user.click(within(dialog).getByTestId('link-confirm-open'));
    await waitFor(() => expect(backend.control.openedUrls()).toEqual(['http://example.com/a', 'https://example.org/target']));
  });

  it('confirms OSC 8 links with the normalized target: punycode host on its own line', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    // A lookalike host (Cyrillic "a") and a right-to-left override in the path.
    const uri = 'https://\u0430pple.example/\u202Egnp.exe';
    act(() => engine.options.onLink({ kind: 'osc8', uri, event: new MouseEvent('click', { ctrlKey: true }) }));
    const dialog = await screen.findByTestId('link-confirm-dialog');
    const href = new URL(uri).href;
    expect(href).toMatch(/^https:\/\/xn--/);
    expect(within(dialog).getByTestId('link-confirm-url')).toHaveTextContent(href);
    expect(within(dialog).getByTestId('link-confirm-url').textContent).not.toMatch(/[^\x20-\x7e]/);
    expect(within(dialog).getByTestId('link-confirm-host')).toHaveTextContent(new URL(uri).host);
    await user.click(within(dialog).getByTestId('link-confirm-open'));
    await waitFor(() => expect(backend.control.openedUrls()).toEqual([href]));
  });

  it('acknowledges the output the terminal processed (flow control)', async () => {
    const { backend } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    const acks = () => backend.control.calls().filter((name) => name === 'session_ack').length;
    engine.holdProcessed = true;
    act(() => backend.control.sessions.output(sessionId, 'z'.repeat(200 * 1024)));
    await waitFor(() => expect(engine.written.length).toBeGreaterThan(200 * 1024));
    expect(acks()).toBe(0);
    act(() => engine.releaseProcessed());
    await waitFor(() => expect(acks()).toBe(1));
  });

  it('copies the selection on select when the setting is on', async () => {
    const { backend } = await withVault(renderUnlocked({ settings: { 'terminal.copy_on_select': true } }));
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    act(() => engine.select('selected text'));
    await waitFor(() => expect(backend.control.clipboard()).toBe('selected text'));

    // Ctrl+Shift+C copies, Ctrl+Shift+A selects all, Ctrl+Shift+V pastes (terminal focused).
    act(() => backend.control.setClipboard(null));
    const textarea = screen.getByTestId('terminal-view').querySelector('textarea') as HTMLTextAreaElement;
    textarea.focus();
    engine.selection = 'copied by key';
    fireEvent.keyDown(textarea, { key: 'C', code: 'KeyC', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(backend.control.clipboard()).toBe('copied by key'));
    fireEvent.keyDown(textarea, { key: 'A', code: 'KeyA', ctrlKey: true, shiftKey: true });
    expect(engine.getSelection()).toContain('PS C:\\mock>');
    act(() => backend.control.setClipboard('whoami'));
    fireEvent.keyDown(textarea, { key: 'V', code: 'KeyV', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toBe('whoami'));
  });

  it('splits with Ctrl+Shift+D / E, focuses panes with Ctrl+Alt+arrows and drags the divider', async () => {
    const { backend } = await withVault(renderUnlocked());
    const { sessionId, tabId } = await openLocal();
    const engine = await engineFor(sessionId);
    await waitFor(() => expect(engine.focused).toBe(true));
    const textarea = screen.getByTestId('terminal-view').querySelector('textarea') as HTMLTextAreaElement;
    textarea.focus();

    fireEvent.keyDown(textarea, { key: 'D', code: 'KeyD', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(screen.getAllByTestId('terminal-pane')).toHaveLength(2));
    const [first, second] = screen.getAllByTestId('terminal-pane');
    expect(second).not.toHaveAttribute('data-session-id');
    expect(second).toHaveAttribute('data-active', 'true');
    expect(first).toHaveAttribute('data-active', 'false');
    // The sidebar acts on the active pane: an empty pane has no session yet.
    act(() => openTerminalSidebar('history'));
    expect(await screen.findByText('Choose a host in this pane first.')).toBeInTheDocument();
    act(() => toggleTerminalSidebar());
    expect(screen.getByTestId('split-divider')).toHaveAttribute('data-direction', 'row');

    fireEvent.keyDown(window, { key: 'ArrowLeft', ctrlKey: true, altKey: true });
    await waitFor(() => expect(screen.getAllByTestId('terminal-pane')[0]).toHaveAttribute('data-active', 'true'));
    fireEvent.keyDown(window, { key: 'E', code: 'KeyE', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(screen.getAllByTestId('terminal-pane')).toHaveLength(3));
    expect(screen.getAllByTestId('split-divider').map((divider) => divider.getAttribute('data-direction'))).toEqual(
      expect.arrayContaining(['row', 'column']),
    );
    fireEvent.keyDown(window, { key: 'ArrowUp', ctrlKey: true, altKey: true });
    await waitFor(() => expect(screen.getAllByTestId('terminal-pane')[0]).toHaveAttribute('data-active', 'true'));

    // Divider drag (the area is 0 px wide in jsdom: give it a size).
    const area = screen.getAllByTestId('terminal-pane')[0]?.parentElement as HTMLElement;
    vi.spyOn(area, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 0, y: 0, width: 1000, height: 600 }));
    const divider = screen.getAllByTestId('split-divider').find((element) => element.getAttribute('data-direction') === 'row') as HTMLElement;
    fireEvent.pointerDown(divider, { button: 0, clientX: 500, clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(divider, { clientX: 700, clientY: 10, pointerId: 1 });
    fireEvent.pointerUp(divider, { pointerId: 1 });
    const root = workspaceStore.get().tabs.find((tab) => tab.id === tabId)?.root;
    expect(root?.type === 'split' && root.sizes[0]).toBeCloseTo(0.7, 5);

    // Escape in an empty pane's host picker closes that pane only.
    const pickers = screen.getAllByTestId('pane-host-picker-search-input');
    fireEvent.keyDown(pickers[0] as HTMLElement, { key: 'Escape' });
    await waitFor(() => expect(screen.getAllByTestId('terminal-pane')).toHaveLength(2));
    expect(sessionOf(backend, sessionId)?.state).toBe('connected');

    // The tab's last session ends: the tab closes with its empty panes (UI 4.6).
    await typeLine(engine, 'exit');
    await waitFor(() => expect(screen.queryAllByTestId('terminal-pane')).toHaveLength(0));
    expect(workspaceStore.get().tabs.find((tab) => tab.id === tabId)).toBeUndefined();
    expect(sessionOf(backend, sessionId)).toBeUndefined();
    expect(splitPane('row', 'no-such-tab')).toBeNull();
  });

  it('keeps the terminals mounted and receiving output while locked', async () => {
    const { backend, password, user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    act(() => backend.control.autoLock());
    await waitFor(() => expect(screen.getByTestId('app-root')).toHaveAttribute('data-lock-state', 'locked'));
    expect(screen.getByTestId('terminal-host')).toHaveAttribute('hidden');
    expect(screen.queryByTestId('titlebar-session-tab')).not.toBeInTheDocument();
    act(() => backend.control.sessions.output(sessionId, 'printed while locked\r\n'));
    await waitFor(() => expect(engine.written).toContain('printed while locked'));
    expect(engine.disposed).toBe(false);
    await user.type(await screen.findByTestId('unlock-password'), password);
    await user.click(screen.getByTestId('unlock-submit'));
    await waitFor(() => expect(screen.getByTestId('app-root')).toHaveAttribute('data-lock-state', 'unlocked'));
    expect(screen.getByTestId('titlebar-session-tab')).toBeInTheDocument();
    activateTab(workspaceStore.get().tabs[0]?.id as string);
    await waitFor(() => expect(screen.getByTestId('terminal-host')).not.toHaveAttribute('hidden'));
    expect(engine.disposed).toBe(false);
  });

  it('rebuilds tabs from the snapshot after a reload and restores the last session', async () => {
    const { backend } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    await engineFor(sessionId);
    // A reload: the window forgets its workspace, the core still has the session.
    act(() => resetWorkspace());
    const snapshot = await backend.invoke('state_snapshot', undefined);
    act(() => applySnapshotSessions(snapshot.sessions, undefined));
    expect(workspaceStore.get().tabs).toHaveLength(1);
    expect(sessionsStore.get().has(sessionId)).toBe(true);
    await engineFor(sessionId);

    // The previous launch's tabs, as the first unlocked snapshot of this launch showed them.
    const previous = {
      saved_at: '2026-10-18T10:00:00.000Z',
      tabs: [{ root: { session: { kind: 'local' } } }, { root: { session: { kind: 'local' } } }],
    };
    act(() =>
      applySnapshotSessions([], undefined, {
        settings: { ...settingsStore.get(), 'session.last': previous },
        launchedAt: '2026-10-19T09:00:00.000Z',
        unlocked: true,
      }),
    );
    await act(async () => {
      await restoreLastSession();
    });
    expect(workspaceStore.get().tabs).toHaveLength(3);
    expect(workspaceStore.get().active).toBe(workspaceStore.get().tabs[1]?.id);
    await waitFor(() => expect(backend.control.sessions.list()).toHaveLength(3));
    // Restore is used once.
    await act(async () => {
      await restoreLastSession();
    });
    expect(workspaceStore.get().tabs).toHaveLength(3);
  });

  it('never shows a session twice when a snapshot arrives while it opens', async () => {
    const { backend } = await withVault(renderUnlocked());
    let opened: Promise<unknown> = Promise.resolve();
    let snapshot: Awaited<ReturnType<typeof backend.invoke<'state_snapshot'>>> | undefined;
    await act(async () => {
      opened = openLocalTerminal();
      // The core already has the session; a resync shows it before the open returns.
      snapshot = await backend.invoke('state_snapshot', undefined);
      applySnapshotSessions(snapshot.sessions, undefined);
      await opened;
    });
    expect(workspaceStore.get().tabs).toHaveLength(1);
    expect(screen.getAllByTestId('terminal-pane')).toHaveLength(1);

    // Opened for a split pane: the snapshot's tab is replaced by the pane.
    const tabId = workspaceStore.get().tabs[0]?.id as string;
    const paneId = splitPane('row', tabId) as string;
    await act(async () => {
      opened = openLocalTerminal({ inPane: { tabId, paneId } });
      snapshot = await backend.invoke('state_snapshot', undefined);
      applySnapshotSessions(snapshot.sessions, undefined);
      await opened;
    });
    expect(workspaceStore.get().tabs).toHaveLength(1);
    expect(screen.getAllByTestId('terminal-pane')).toHaveLength(2);
    expect(screen.getAllByTestId('terminal-pane').every((pane) => pane.hasAttribute('data-session-id'))).toBe(true);
  });

  it('closes a session whose pane was closed while it opened', async () => {
    const { backend } = await withVault(renderUnlocked());
    const { tabId } = await openLocal();
    const paneId = splitPane('row', tabId) as string;
    await act(async () => {
      const opened = openLocalTerminal({ inPane: { tabId, paneId } });
      closePane(paneId);
      expect(await opened).toBeNull();
    });
    expect(workspaceStore.get().tabs).toHaveLength(1);
    await waitFor(() => expect(backend.control.sessions.list()).toHaveLength(1));
  });

  it('opens the intent of a new window', async () => {
    const { backend } = await withVault(renderUnlocked());
    act(() => applySnapshotSessions([], { kind: 'open_local', shell_id: 'cmd' }));
    await waitFor(() => expect(backend.control.sessions.list()).toHaveLength(1));
    expect(backend.control.sessions.list()[0]).toMatchObject({ kind: 'local', shell_id: 'cmd', title: 'Command Prompt' });
  });

  it('installs the terminal shortcuts only for session tabs', async () => {
    await withVault(renderUnlocked());
    const stop = installTerminalShortcuts('windows');
    fireEvent.keyDown(window, { key: 'D', code: 'KeyD', ctrlKey: true, shiftKey: true });
    expect(workspaceStore.get().tabs).toEqual([]);
    stop();
  });
});
