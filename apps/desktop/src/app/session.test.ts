// Window session: snapshot protocol, events from other windows, activity and
// shortcuts.

import { act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { newPassword, resetApp, setupBackend } from '../test/app';
import { installActivityReporter } from './activity';
import { startSession } from './session';
import { installShortcuts } from './shortcuts';
import { getToasts } from './toast';
import { lockStore, notificationsStore, settingsStore, vaultsStore, viewStore, windowStore } from './store';

let stop: (() => void) | null = null;

afterEach(() => {
  stop?.();
  stop = null;
  vi.useRealTimers();
  resetApp();
});

async function boot(options: Parameters<typeof setupBackend>[0]) {
  const backend = setupBackend(options);
  stop = startSession();
  await waitFor(() => expect(windowStore.get().phase).toBe('ready'));
  return backend;
}

describe('window session', () => {
  it('fills the stores from the snapshot', async () => {
    await boot({ initialState: 'unlocked', password: newPassword(), windowLabel: 'w-2' });
    expect(windowStore.get().label).toBe('w-2');
    expect(lockStore.get()?.state).toBe('unlocked');
    expect(vaultsStore.get()).toHaveLength(1);
    expect(settingsStore.get()['security.auto_lock_minutes']).toBe(15);
    expect(notificationsStore.get()).toEqual({ items: [], unread: 0 });
  });

  it('follows changes made by other windows: settings, lock, notifications', async () => {
    const password = newPassword();
    const backend = await boot({ initialState: 'unlocked', password });
    await act(async () => {
      await backend.invoke('settings_set', { values: { 'view.hosts.layout': 'list' } });
    });
    await waitFor(() => expect(settingsStore.get()['view.hosts.layout']).toBe('list'));

    await act(async () => {
      backend.control.notify('notifications.title');
    });
    await waitFor(() => expect(notificationsStore.get().unread).toBe(1));
    expect(getToasts().map((toast) => toast.testId)).toEqual(['notification-toast']);

    await act(async () => {
      await backend.invoke('lock', undefined);
    });
    await waitFor(() => expect(lockStore.get()?.state).toBe('locked'));
    expect(vaultsStore.get()).toEqual([]);
    expect(getToasts()).toEqual([]);

    // No toasts while locked; the counter still follows.
    await act(async () => {
      backend.control.notify('notifications.title');
    });
    await waitFor(() => expect(notificationsStore.get().unread).toBe(2));
    expect(getToasts()).toEqual([]);

    await act(async () => {
      await backend.invoke('unlock', { password });
    });
    await waitFor(() => expect(lockStore.get()?.state).toBe('unlocked'));
    expect(vaultsStore.get()).toHaveLength(1);
  });

  it('keeps first run on screen until Finish while the core is unlocked', async () => {
    const backend = await boot({});
    expect(lockStore.get()?.state).toBe('first_run');
    viewStore.set({ ...viewStore.get(), firstRunInProgress: true });
    await act(async () => {
      await backend.invoke('first_run_setup', { password: newPassword() });
    });
    await waitFor(() => expect(lockStore.get()?.state).toBe('unlocked'));
    expect(viewStore.get().firstRunInProgress).toBe(true);
  });

  it('reports a startup error', async () => {
    setupBackend({ startupError: 'storage_error' });
    stop = startSession();
    await waitFor(() => expect(windowStore.get().phase).toBe('error'));
    expect(windowStore.get().errorCode).toBe('storage_error');
  });
});

describe('activity reporting', () => {
  it('reports input at most every 30 seconds', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    const backend = setupBackend({ initialState: 'unlocked', password: newPassword() });
    const uninstall = installActivityReporter();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    window.dispatchEvent(new MouseEvent('pointermove'));
    await waitFor(() => expect(backend.control.activityCount()).toBe(1));
    vi.setSystemTime(Date.now() + 29_000);
    window.dispatchEvent(new MouseEvent('pointerdown'));
    vi.setSystemTime(Date.now() + 2_000);
    window.dispatchEvent(new WheelEvent('wheel'));
    await waitFor(() => expect(backend.control.activityCount()).toBe(2));
    uninstall();
    vi.setSystemTime(Date.now() + 60_000);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    await Promise.resolve();
    expect(backend.control.activityCount()).toBe(2);
  });

  it('reports input inside the 30 seconds once more when they end', async () => {
    vi.useFakeTimers();
    const backend = setupBackend({ initialState: 'unlocked', password: newPassword() });
    const uninstall = installActivityReporter();
    const type = () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    type();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.control.activityCount()).toBe(1);
    // Typing until t = 29 s: one trailing report at t = 30 s.
    await vi.advanceTimersByTimeAsync(10_000);
    type();
    await vi.advanceTimersByTimeAsync(19_000);
    type();
    expect(backend.control.activityCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(backend.control.activityCount()).toBe(2);
    // No input: nothing more.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(backend.control.activityCount()).toBe(2);
    // A pending trailing report is dropped with the reporter.
    type();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.control.activityCount()).toBe(3);
    type();
    uninstall();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(backend.control.activityCount()).toBe(3);
  });
});

describe('shortcuts', () => {
  it('binds Ctrl+1 and Ctrl+, and nothing for hidden features or AltGr', async () => {
    await boot({ initialState: 'unlocked', password: newPassword() });
    const uninstall = installShortcuts('windows');
    const press = (init: KeyboardEventInit) => {
      const event = new KeyboardEvent('keydown', { cancelable: true, ...init });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    };
    expect(press({ key: ',', ctrlKey: true })).toBe(true);
    expect(viewStore.get().settingsOpen).toBe(true);
    expect(press({ key: '1', ctrlKey: true })).toBe(true);
    expect(press({ key: ',', ctrlKey: true, altKey: true })).toBe(false);
    for (const key of ['2', 'l', 'L', 'k', 'p']) {
      expect(press({ key, ctrlKey: true })).toBe(false);
      expect(press({ key, ctrlKey: true, shiftKey: true })).toBe(false);
    }
    expect(press({ key: ',', metaKey: true })).toBe(false);
    uninstall();

    const uninstallMac = installShortcuts('macos');
    viewStore.set({ ...viewStore.get(), settingsOpen: false });
    expect(press({ key: ',', ctrlKey: true })).toBe(false);
    expect(press({ key: ',', metaKey: true })).toBe(true);
    expect(viewStore.get().settingsOpen).toBe(true);
    uninstallMac();
  });
});
