// Toasts (UI 1.5, 2.4): duration, action button, dismissal.

import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clearToasts, showToast } from '../toast';
import { ToastRegion } from './ToastRegion';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => clearToasts());
  vi.useRealTimers();
});

describe('ToastRegion', () => {
  it('shows a toast for 5 seconds by default', () => {
    render(<ToastRegion />);
    act(() => {
      showToast({ message: 'Saved', testId: 'notification-toast', dataId: 7, level: 'success' });
    });
    const toast = screen.getByTestId('notification-toast');
    expect(toast).toHaveTextContent('Saved');
    expect(toast).toHaveAttribute('data-id', '7');
    expect(toast).toHaveAttribute('data-level', 'success');
    act(() => {
      vi.advanceTimersByTime(4900);
    });
    expect(screen.getByTestId('notification-toast')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(200);
    });
    expect(screen.queryByTestId('notification-toast')).not.toBeInTheDocument();
  });

  it('runs the action once and removes the toast', () => {
    const onAction = vi.fn();
    render(<ToastRegion />);
    act(() => {
      showToast({
        message: 'Host removed',
        durationMs: 8000,
        testId: 'hosts-undo-toast',
        action: { label: 'Undo', onAction, testId: 'hosts-undo-toast-action' },
      });
    });
    act(() => {
      vi.advanceTimersByTime(7000);
    });
    act(() => {
      screen.getByTestId('hosts-undo-toast-action').click();
    });
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('hosts-undo-toast')).not.toBeInTheDocument();
  });

  it('dismisses a toast with its close button and alerts errors', () => {
    render(<ToastRegion />);
    act(() => {
      showToast({ message: 'Failed', level: 'error', testId: 'app-error-toast' });
    });
    expect(screen.getByRole('alert')).toHaveTextContent('Failed');
    act(() => {
      screen.getByRole('button', { name: 'Dismiss' }).click();
    });
    expect(screen.queryByTestId('app-error-toast')).not.toBeInTheDocument();
    expect(screen.getByTestId('toast-region')).toBeEmptyDOMElement();
  });
});
