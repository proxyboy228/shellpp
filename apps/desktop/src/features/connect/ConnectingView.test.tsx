// The connection screen (UI 4.8): steps, Cancel, every prompt kind, host key
// dialogs and the failure states. Secrets are generated at run time.

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clearToasts, getToasts } from '../../app/toast';
import { IpcError, setBackend, type SessionInfo, type SessionPrompt } from '../../ipc';
import { ConnectingView } from './ConnectingView';
import { createRecordingBackend, type RecordingBackend } from './testing/recordingBackend';

let backend: RecordingBackend;

beforeEach(() => {
  backend = createRecordingBackend({
    session_reconnect: () => session(),
  });
  setBackend(backend);
});

afterEach(() => {
  setBackend(null);
});

const SESSION_ID = '01990000-0000-7000-8000-0000000000c1';

function session(patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session_id: SESSION_ID,
    rev: 1,
    kind: 'ssh',
    window: 'w-1',
    state: 'connecting',
    step: 'connecting',
    title: 'Lab server',
    target: { kind: 'host', vault_id: '01990000-0000-7000-8000-00000000aaaa', record_id: '01990000-0000-7000-8000-0000000000b1' },
    address: '192.0.2.20',
    port: 2222,
    username: 'ops',
    terminal_theme: { builtin: 'graphite' },
    backspace: 'default',
    record_history: true,
    ...patch,
  };
}

function withPrompt(prompt: SessionPrompt, patch: Partial<SessionInfo> = {}): SessionInfo {
  return session({ step: 'authenticating', prompt, ...patch });
}

const secret = () => `s-${crypto.randomUUID()}`;

describe('ConnectingView: connecting', () => {
  it('shows the title, the user@host:port address, the step and the connecting status', () => {
    render(<ConnectingView session={session({ step: 'host_key' })} onClose={vi.fn()} />);
    const view = screen.getByTestId('connecting-view');
    expect(view).toHaveAttribute('data-session-id', SESSION_ID);
    expect(view).toHaveAttribute('data-state', 'connecting');
    expect(view).toHaveAttribute('data-step', 'host_key');
    expect(screen.getByTestId('connecting-title')).toHaveTextContent('Lab server');
    expect(screen.getByTestId('connecting-address')).toHaveTextContent('ops@192.0.2.20:2222');
    expect(screen.getByTestId('connecting-step')).toHaveTextContent('Checking the host key');
    expect(screen.getByTestId('connecting-status')).toHaveAccessibleName('Connecting');
  });

  it('omits port 22 and brackets IPv6 addresses with a port', () => {
    const { rerender } = render(<ConnectingView session={session({ port: 22, username: undefined })} onClose={vi.fn()} />);
    expect(screen.getByTestId('connecting-address')).toHaveTextContent(/^192\.0\.2\.20$/);
    rerender(<ConnectingView session={session({ address: '2001:db8::9', port: 2200 })} onClose={vi.fn()} />);
    expect(screen.getByTestId('connecting-address')).toHaveTextContent('ops@[2001:db8::9]:2200');
  });

  it('cancels the attempt', async () => {
    const user = userEvent.setup();
    render(<ConnectingView session={session()} onClose={vi.fn()} />);
    await user.click(screen.getByTestId('connecting-cancel'));
    expect(backend.callsOf('session_cancel')).toEqual([{ session_id: SESSION_ID }]);
  });
});

describe('ConnectingView: prompts', () => {
  it('asks for a username and sends it as text', async () => {
    const user = userEvent.setup();
    render(<ConnectingView session={withPrompt({ kind: 'username', prompt_id: 4 })} onClose={vi.fn()} />);
    expect(screen.getByTestId('session-prompt')).toHaveAttribute('data-kind', 'username');
    const input = screen.getByTestId('session-prompt-input');
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute('type', 'text');
    expect(screen.queryByTestId('session-prompt-save')).not.toBeInTheDocument();
    // The form's own Cancel ends the attempt; the screen's Cancel is not repeated.
    expect(screen.queryByTestId('connecting-cancel')).not.toBeInTheDocument();
    // An empty username is not sent.
    await user.keyboard('{Enter}');
    expect(backend.callsOf('session_prompt_reply')).toEqual([]);
    await user.type(input, 'deploy{Enter}');
    expect(backend.callsOf('session_prompt_reply')).toEqual([
      { session_id: SESSION_ID, prompt_id: 4, reply: { kind: 'text', value: 'deploy' } },
    ]);
  });

  it('masks a password, offers Save (off by default) and clears the value after sending', async () => {
    const user = userEvent.setup();
    const password = secret();
    render(
      <ConnectingView
        session={withPrompt({ kind: 'password', prompt_id: 7, username: 'ops', error: false, can_save: true })}
        onClose={vi.fn()}
      />,
    );
    const input = screen.getByTestId('session-prompt-input');
    expect(input).toHaveAttribute('type', 'password');
    expect(screen.getByTestId('session-prompt-label')).toHaveTextContent('Password for ops');
    expect(screen.getByTestId('session-prompt-input-toggle')).toBeInTheDocument();
    const save = screen.getByTestId('session-prompt-save');
    expect(save).not.toBeChecked();
    await user.type(input, password);
    await user.click(save);
    await user.click(screen.getByTestId('session-prompt-continue'));
    expect(backend.callsOf('session_prompt_reply')).toEqual([
      { session_id: SESSION_ID, prompt_id: 7, reply: { kind: 'text', value: password, save: true } },
    ]);
    expect(input).toHaveValue('');
  });

  it('shows a wrong password error in the same form and hides Save for a quick connect', () => {
    render(
      <ConnectingView
        session={withPrompt({ kind: 'password', prompt_id: 8, error: true, can_save: false }, { target: { kind: 'quick', address: '192.0.2.20' } })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByTestId('session-prompt-input-error')).toHaveTextContent('Wrong password');
    expect(screen.queryByTestId('session-prompt-save')).not.toBeInTheDocument();
  });

  it('asks for a key passphrase with the key label', async () => {
    const user = userEvent.setup();
    const passphrase = secret();
    render(
      <ConnectingView
        session={withPrompt({ kind: 'passphrase', prompt_id: 9, key_label: 'deploy key', error: false, can_save: true })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByTestId('session-prompt')).toHaveAttribute('data-kind', 'passphrase');
    expect(screen.getByTestId('session-prompt-label')).toHaveTextContent('deploy key');
    await user.type(screen.getByTestId('session-prompt-input'), `${passphrase}{Enter}`);
    expect(backend.callsOf('session_prompt_reply')).toEqual([
      { session_id: SESSION_ID, prompt_id: 9, reply: { kind: 'text', value: passphrase, save: false } },
    ]);
  });

  it('shows keyboard-interactive server texts in a frame, one question at a time, masked unless echoed', async () => {
    const user = userEvent.setup();
    const code = secret();
    const { rerender } = render(
      <ConnectingView
        session={withPrompt({
          kind: 'keyboard_interactive',
          prompt_id: 11,
          name: 'Two-step check',
          instruction: '<b>Enter the code</b>',
          prompt: 'Verification code:',
          echo: false,
          index: 0,
          count: 2,
          error: false,
        })}
        onClose={vi.fn()}
      />,
    );
    const frame = screen.getByTestId('session-prompt-server-text');
    expect(frame).toHaveTextContent('Sent by the server Lab server');
    expect(frame).toHaveTextContent('Two-step check');
    // Server text is plain text, never markup.
    expect(frame).toHaveTextContent('<b>Enter the code</b>');
    expect(frame.querySelector('b')).toBeNull();
    expect(screen.getByTestId('session-prompt-label')).toHaveTextContent('Verification code:');
    expect(screen.getByTestId('session-prompt-progress')).toHaveTextContent('Question 1 of 2');
    expect(screen.getByTestId('session-prompt-input')).toHaveAttribute('type', 'password');
    expect(screen.queryByTestId('session-prompt-save')).not.toBeInTheDocument();
    await user.type(screen.getByTestId('session-prompt-input'), `${code}{Enter}`);
    expect(backend.callsOf('session_prompt_reply')).toEqual([
      { session_id: SESSION_ID, prompt_id: 11, reply: { kind: 'text', value: code } },
    ]);

    rerender(
      <ConnectingView
        session={withPrompt({
          kind: 'keyboard_interactive',
          prompt_id: 12,
          name: '',
          instruction: '',
          prompt: 'Favourite colour:',
          echo: true,
          index: 1,
          count: 2,
          error: false,
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByTestId('session-prompt-progress')).toHaveTextContent('Question 2 of 2');
    expect(screen.getByTestId('session-prompt-input')).toHaveAttribute('type', 'text');
    expect(screen.getByTestId('session-prompt-input')).toHaveValue('');
  });

  it('Cancel in a prompt sends a cancel reply', async () => {
    const user = userEvent.setup();
    render(<ConnectingView session={withPrompt({ kind: 'username', prompt_id: 5 })} onClose={vi.fn()} />);
    await user.click(screen.getByTestId('session-prompt-cancel'));
    expect(backend.callsOf('session_prompt_reply')).toEqual([{ session_id: SESSION_ID, prompt_id: 5, reply: { kind: 'cancel' } }]);
  });

  it('renders the host key dialog and replies with the decision', async () => {
    const user = userEvent.setup();
    render(
      <ConnectingView
        session={withPrompt(
          {
            kind: 'host_key_unknown',
            prompt_id: 2,
            key: { address: '192.0.2.20', port: 2222, key_type: 'ssh-ed25519', fingerprint: 'SHA256:testFingerprint' },
          },
          { step: 'host_key' },
        )}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByTestId('host-key-dialog')).toHaveAttribute('data-kind', 'unknown');
    expect(screen.queryByTestId('session-prompt')).not.toBeInTheDocument();
    // The dialog's Cancel is the only Cancel at the host key step (UI 10.2).
    expect(screen.queryByTestId('connecting-cancel')).not.toBeInTheDocument();
    expect(screen.getByTestId('host-key-dialog-accept')).toHaveFocus();
    await user.click(screen.getByTestId('host-key-dialog-once'));
    expect(backend.callsOf('session_prompt_reply')).toEqual([
      { session_id: SESSION_ID, prompt_id: 2, reply: { kind: 'host_key', decision: 'connect_once' } },
    ]);
  });

  it('takes focus only in the active pane, and when the pane becomes active', () => {
    const elsewhere = document.createElement('input');
    document.body.appendChild(elsewhere);
    elsewhere.focus();
    const hostKey = withPrompt(
      {
        kind: 'host_key_unknown',
        prompt_id: 3,
        key: { address: '192.0.2.20', port: 2222, key_type: 'ssh-ed25519', fingerprint: 'SHA256:testFingerprint' },
      },
      { step: 'host_key' },
    );
    const { rerender, unmount } = render(<ConnectingView session={hostKey} onClose={vi.fn()} focused={false} />);
    expect(elsewhere).toHaveFocus();
    rerender(<ConnectingView session={hostKey} onClose={vi.fn()} focused />);
    expect(screen.getByTestId('host-key-dialog-accept')).toHaveFocus();
    unmount();

    elsewhere.focus();
    const password = withPrompt({ kind: 'password', prompt_id: 4, username: 'ops', error: false, can_save: false });
    const second = render(<ConnectingView session={password} onClose={vi.fn()} focused={false} />);
    expect(elsewhere).toHaveFocus();
    second.rerender(<ConnectingView session={password} onClose={vi.fn()} focused />);
    expect(screen.getByTestId('session-prompt-input')).toHaveFocus();
    second.unmount();
    elsewhere.remove();
  });
});

describe('ConnectingView: failure', () => {
  it('shows the SSH reason with Retry and Close', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ConnectingView session={session({ state: 'failed', step: undefined, error: { code: 'ssh_error', reason: 'refused' } })} onClose={onClose} />);
    const failed = screen.getByTestId('connect-failed');
    expect(failed).toHaveAttribute('data-code', 'ssh_error');
    expect(failed).toHaveAttribute('data-reason', 'refused');
    expect(screen.getByTestId('connect-failed-reason')).toHaveTextContent('The connection was refused.');
    expect(screen.queryByTestId('connecting-cancel')).not.toBeInTheDocument();
    expect(screen.queryByTestId('connecting-step')).not.toBeInTheDocument();
    expect(screen.getByTestId('connecting-status')).toHaveAccessibleName('Connection failed');
    await user.click(screen.getByTestId('connect-failed-retry'));
    expect(backend.callsOf('session_reconnect')).toEqual([{ session_id: SESSION_ID }]);
    await user.click(screen.getByTestId('connect-failed-close'));
    expect(onClose).toHaveBeenCalled();
  });

  it('uses errors.<code> for other codes', () => {
    render(<ConnectingView session={session({ state: 'failed', error: { code: 'cancelled' } })} onClose={vi.fn()} />);
    expect(screen.getByTestId('connect-failed')).toHaveAttribute('data-code', 'cancelled');
    expect(screen.getByTestId('connect-failed-reason')).toHaveTextContent('The operation was cancelled.');
  });

  it('shows the revoked key warning with Close only', () => {
    render(
      <ConnectingView
        session={session({
          state: 'failed',
          error: { code: 'host_key_revoked' },
          host_key: { address: '192.0.2.20', port: 2222, key_type: 'ssh-ed25519', fingerprint: 'SHA256:revokedTest' },
        })}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByTestId('host-key-revoked')).toBeInTheDocument();
    expect(screen.getByTestId('host-key-revoked-fingerprint')).toHaveTextContent('SHA256:revokedTest');
    expect(screen.queryByTestId('connect-failed-retry')).not.toBeInTheDocument();
    expect(screen.getByTestId('connect-failed-close')).toBeInTheDocument();
  });

  it('reports a Retry that the core refuses', async () => {
    const user = userEvent.setup();
    backend.failNext('session_reconnect', new IpcError('wrong_state'));
    render(<ConnectingView session={session({ state: 'failed', error: { code: 'ssh_error', reason: 'timeout' } })} onClose={vi.fn()} />);
    await user.click(screen.getByTestId('connect-failed-retry'));
    await waitFor(() => expect(getToasts().map((toast) => toast.testId)).toContain('app-error-toast'));
    expect(backend.callsOf('session_reconnect')).toHaveLength(1);
    clearToasts();
  });
});
