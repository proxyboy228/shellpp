// Notification center (UI 1.5): counter, list newest first, mark read, toasts.

import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUnlocked, resetApp } from '../../test/app';

vi.mock('../../features/hosts', () => ({
  HostsScreen: () => <section data-testid="hosts-screen" />,
}));

afterEach(() => {
  resetApp();
});

describe('NotificationCenter', () => {
  it('shows the empty list without a counter', async () => {
    const { user } = await renderUnlocked();
    expect(screen.queryByTestId('titlebar-notifications-badge')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('titlebar-notifications-button'));
    expect(screen.getByTestId('notifications-popover')).toBeInTheDocument();
    expect(screen.getByTestId('notifications-empty')).toBeInTheDocument();
  });

  it('counts unread notifications, shows a toast for each new one and lists them newest first', async () => {
    const { backend, user } = await renderUnlocked();
    await act(async () => {
      backend.control.notify('about.version', { version: 'first' });
    });
    const toast = await screen.findByTestId('notification-toast');
    expect(toast).toHaveAttribute('data-id', '1');
    expect(toast).toHaveTextContent('Version first');
    await act(async () => {
      backend.control.notify('about.version', { version: 'second' }, 'warning');
    });
    await waitFor(() => expect(screen.getByTestId('titlebar-notifications-badge')).toHaveTextContent('2'));
    expect(screen.getAllByTestId('notification-toast')).toHaveLength(2);

    await user.click(screen.getByTestId('titlebar-notifications-button'));
    const items = within(screen.getByTestId('notifications-popover')).getAllByTestId('notifications-item');
    expect(items.map((item) => item.getAttribute('data-id'))).toEqual(['2', '1']);
    expect(items[0]).toHaveTextContent('Version second');
    // Opening the list marks everything read.
    await waitFor(() => expect(screen.queryByTestId('titlebar-notifications-badge')).not.toBeInTheDocument());
    await waitFor(() =>
      expect(screen.getAllByTestId('notifications-item').map((item) => item.getAttribute('data-read'))).toEqual(['true', 'true']),
    );

    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('notifications-popover')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-notifications-button')).toHaveFocus();
  });

  it('closes on a press outside', async () => {
    const { user } = await renderUnlocked();
    await user.click(screen.getByTestId('titlebar-notifications-button'));
    await user.click(screen.getByTestId('hosts-screen'));
    expect(screen.queryByTestId('notifications-popover')).not.toBeInTheDocument();
  });
});
