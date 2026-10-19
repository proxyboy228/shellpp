// The overlay of a reconnect attempt in progress (UI 4.6, 4.8): the step,
// Cancel (stops the attempt and the automatic attempts) and Close.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setBackend, type SessionInfo } from '../../ipc';
import { createRecordingBackend, type RecordingBackend } from '../connect/testing/recordingBackend';
import { SessionReconnectingOverlay } from './SessionOverlays';

const SESSION_ID = '01990000-0000-7000-8000-0000000000e1';

let backend: RecordingBackend;

beforeEach(() => {
  backend = createRecordingBackend();
  setBackend(backend);
});

afterEach(() => {
  setBackend(null);
});

function reconnecting(patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: SESSION_ID,
    rev: 4,
    kind: 'ssh',
    window: 'w-1',
    state: 'connecting',
    step: 'authenticating',
    title: 'Lab server',
    terminal_theme: { builtin: 'graphite' },
    backspace: 'default',
    record_history: true,
    reconnect: { attempt: 2, waiting_for_unlock: false, in_progress: true },
    ...patch,
  };
}

describe('SessionReconnectingOverlay', () => {
  it('shows the attempt and its step, and cancels or closes', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<SessionReconnectingOverlay session={reconnecting()} onClose={onClose} />);
    const overlay = screen.getByTestId('session-reconnecting');
    expect(overlay).toHaveAttribute('data-step', 'authenticating');
    expect(overlay).toHaveTextContent('2');
    expect(screen.getByTestId('session-reconnecting-step')).toHaveTextContent('Authenticating');
    await user.click(screen.getByTestId('session-reconnecting-cancel'));
    expect(backend.callsOf('session_cancel')).toEqual([{ session_id: SESSION_ID }]);
    await user.click(screen.getByTestId('session-reconnecting-close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('also covers Reconnect after an end (no automatic attempt)', () => {
    render(<SessionReconnectingOverlay session={reconnecting({ reconnect: undefined, step: 'connecting' })} onClose={vi.fn()} />);
    expect(screen.getByTestId('session-reconnecting-step')).toHaveTextContent('Connecting to the server');
    expect(screen.getByTestId('session-reconnecting-cancel')).toBeInTheDocument();
  });
});
