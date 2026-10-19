// Terminal sidebar (UI 4.2): Tools search, History (newest first, insert
// without Enter, delete, clear), Appearance (font settings, themes saved on
// the host).

import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUnlocked, resetApp } from '../../test/app';
import { settingsStore } from '../../app/store';
import { openTerminalSidebar } from './api';
import { frequentCommands } from './historyView';
import { engineFor, openHost, openLocal, savedHost, typeLine, waitForState, withVault } from './testing/helpers';

vi.mock('../hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

describe('terminal sidebar', () => {
  it('searches the terminal output with previous and next (Ctrl+Shift+F focuses the field)', async () => {
    const { user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    await typeLine(engine, 'lines 3');
    await waitFor(() => expect(engine.written).toContain('line 3'));
    const textarea = screen.getByTestId('terminal-view').querySelector('textarea') as HTMLTextAreaElement;
    textarea.focus();
    fireEvent.keyDown(textarea, { key: 'F', code: 'KeyF', ctrlKey: true, shiftKey: true });
    const input = await screen.findByTestId('terminal-search-input');
    await waitFor(() => expect(input).toHaveFocus());
    expect(screen.getByTestId('terminal-sidebar')).toHaveAttribute('data-tab', 'tools');
    expect(screen.queryByTestId('terminal-autocomplete')).not.toBeInTheDocument();

    await user.type(input, 'line ');
    await waitFor(() => expect(screen.getByTestId('terminal-search-results')).toHaveAttribute('data-count', '3'));
    expect(screen.getByTestId('terminal-search-results')).toHaveTextContent('1 of 3');
    await user.click(screen.getByTestId('terminal-search-next'));
    await user.click(screen.getByTestId('terminal-search-prev'));
    expect(engine.searches.slice(-2)).toEqual([
      { direction: 'next', term: 'line ' },
      { direction: 'previous', term: 'line ' },
    ]);
  });

  it('lists the history newest first, inserts without Enter, deletes and clears', async () => {
    const { backend, user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    await typeLine(engine, 'echo one');
    await typeLine(engine, 'echo two');
    act(() => openTerminalSidebar('history'));
    const list = await screen.findByTestId('history-list');
    const commands = () => within(screen.getByTestId('history-list')).getAllByTestId('history-item-command').map((item) => item.textContent);
    await waitFor(() => expect(commands()).toEqual(['echo two', 'echo one']));
    expect(screen.queryByTestId('history-snippet-hint')).not.toBeInTheDocument();

    const inputBefore = backend.control.sessions.input(sessionId);
    await user.click(within(list).getAllByTestId('history-item-command')[1] as HTMLElement);
    await waitFor(() => expect(backend.control.sessions.input(sessionId)).toBe(`${inputBefore}echo one`));
    // Injected input is never recorded: Enter after the insert adds no entry.
    act(() => engine.type('\r'));
    await waitFor(() => expect(engine.written).toMatch(/echo one\r\none\r\n/));

    await user.type(screen.getByTestId('history-search-input'), 'two');
    await waitFor(() => expect(commands()).toEqual(['echo two']));
    await user.clear(screen.getByTestId('history-search-input'));
    await waitFor(() => expect(commands()).toEqual(['echo two', 'echo one']));

    await user.click(within(screen.getByTestId('history-list')).getAllByTestId('history-item-delete')[0] as HTMLElement);
    await waitFor(() => expect(commands()).toEqual(['echo one']));

    await user.click(screen.getByTestId('history-clear'));
    const dialog = await screen.findByTestId('history-clear-dialog');
    expect(dialog).toHaveTextContent('local terminals');
    await user.click(within(dialog).getByTestId('history-clear-dialog-confirm'));
    await waitFor(() => expect(screen.getByTestId('history-empty')).toBeInTheDocument());
    expect(await backend.invoke('history_list', {})).toEqual([]);
  });

  it('changes the font settings from the Font row', async () => {
    const { user } = await withVault(renderUnlocked());
    const { sessionId } = await openLocal();
    const engine = await engineFor(sessionId);
    act(() => openTerminalSidebar('appearance'));
    expect(screen.getByTestId('appearance-font')).toHaveTextContent('JetBrains Mono, 14');
    await user.click(screen.getByTestId('appearance-font'));
    expect(screen.getByTestId('appearance-font-panel')).toBeInTheDocument();
    await user.click(screen.getByTestId('appearance-font-size-increase'));
    await waitFor(() => expect(settingsStore.get()['terminal.font_size']).toBe(15));
    await waitFor(() => expect(engine.options.fontSize).toBe(15));
    await user.click(screen.getByTestId('appearance-font-size-decrease'));
    await waitFor(() => expect(settingsStore.get()['terminal.font_size']).toBe(14));

    const family = screen.getByTestId('appearance-font-family');
    await user.clear(family);
    await user.type(family, 'Cascadia Mono{Enter}');
    await waitFor(() => expect(settingsStore.get()['terminal.font_family']).toBe('Cascadia Mono'));
    await waitFor(() => expect(engine.options.fontFamily).toBe('Cascadia Mono'));

    const lineHeight = screen.getByTestId('appearance-line-height');
    await user.clear(lineHeight);
    await user.type(lineHeight, '3{Enter}');
    expect(await screen.findByTestId('appearance-line-height-error')).toBeInTheDocument();
    await user.clear(lineHeight);
    await user.type(lineHeight, '1.5{Enter}');
    await waitFor(() => expect(settingsStore.get()['terminal.line_height']).toBe(1.5));
    expect(screen.queryByTestId('appearance-line-height-error')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('appearance-font-back'));
    expect(screen.getByTestId('appearance-font')).toHaveTextContent('Cascadia Mono, 14');
  });

  it('applies a theme to the session and saves it on the saved host', async () => {
    const { backend, vaultId, user } = await withVault(renderUnlocked());
    const recordId = await savedHost(backend, vaultId);
    const { sessionId } = await openHost(vaultId, recordId);
    await waitForState(backend, sessionId, 'connected');
    const engine = await engineFor(sessionId);
    act(() => openTerminalSidebar('appearance'));
    const options = within(screen.getByTestId('appearance-theme-list')).getAllByTestId('appearance-theme-option');
    expect(options.map((option) => option.getAttribute('data-theme'))).toEqual(['graphite', 'paper']);
    expect(options[0]).toHaveAttribute('aria-selected', 'true');
    expect(within(options[1] as HTMLElement).getByTestId('appearance-theme-option-preview')).toBeInTheDocument();
    await user.click(options[1] as HTMLElement);
    await waitFor(() => expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-theme', 'paper'));
    await waitFor(() => expect(engine.options.theme.name).toBe('Paper'));
    await waitFor(async () => {
      const view = await backend.invoke('record_get', { vault_id: vaultId, record_id: recordId });
      expect(view.type === 'host' && view.data.terminal_theme).toEqual({ builtin: 'paper' });
    });
  });

  it('suggests only commands used more than once for the snippet card', () => {
    const entry = (command: string, index: number) => ({
      vault_id: 'v',
      record_id: `r-${index}`,
      command,
      executed_at: `2026-10-19T10:00:0${index}.000Z`,
    });
    expect(frequentCommands(['ls', 'ls', 'git status', 'git status', 'git status', 'pwd'].map(entry))).toEqual(['git status', 'ls']);
  });
});
