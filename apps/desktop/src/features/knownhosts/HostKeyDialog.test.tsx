// Host key dialogs (UI 10.2): first connection, changed key, revoked key.
// Fingerprints are made-up test values.

import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { HostKeyChangedPrompt, HostKeyUnknownPrompt } from '../../ipc';
import { HostKeyDialog, HostKeyRevokedWarning } from './HostKeyDialog';

const KEY = { address: '192.0.2.15', port: 2222, key_type: 'ssh-ed25519', fingerprint: 'SHA256:presentedTESTfingerprint' };

const unknown: HostKeyUnknownPrompt = { kind: 'host_key_unknown', key: KEY };
const changed: HostKeyChangedPrompt = {
  kind: 'host_key_changed',
  key: KEY,
  old_key_type: 'ecdsa-sha2-nistp256',
  old_fingerprint: 'SHA256:savedTESTfingerprint',
};

describe('HostKeyDialog: first connection (TOFU)', () => {
  it('shows the address in OpenSSH form, the key type and the fingerprint', () => {
    render(<HostKeyDialog prompt={unknown} hostTitle="lab" onDecision={vi.fn()} />);
    const dialog = screen.getByTestId('host-key-dialog');
    expect(dialog).toHaveAttribute('role', 'alertdialog');
    expect(dialog).toHaveAttribute('data-kind', 'unknown');
    expect(screen.getByTestId('host-key-dialog-address')).toHaveTextContent('[192.0.2.15]:2222');
    expect(screen.getByTestId('host-key-dialog-key-type')).toHaveTextContent('ssh-ed25519');
    expect(screen.getByTestId('host-key-dialog-fingerprint')).toHaveTextContent('SHA256:presentedTESTfingerprint');
    expect(screen.queryByTestId('host-key-dialog-old-fingerprint')).not.toBeInTheDocument();
    expect(screen.queryByTestId('host-key-dialog-replace')).not.toBeInTheDocument();
  });

  it('focuses Accept and save and sends each decision', async () => {
    const user = userEvent.setup();
    const onDecision = vi.fn();
    render(<HostKeyDialog prompt={unknown} hostTitle="lab" onDecision={onDecision} />);
    expect(screen.getByTestId('host-key-dialog-accept')).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.click(screen.getByTestId('host-key-dialog-once'));
    await user.click(screen.getByTestId('host-key-dialog-cancel'));
    expect(onDecision.mock.calls.map((call) => call[0])).toEqual(['accept_save', 'connect_once', 'cancel']);
  });

  it('writes port 22 addresses without brackets', () => {
    render(<HostKeyDialog prompt={{ ...unknown, key: { ...KEY, port: 22 } }} hostTitle="lab" onDecision={vi.fn()} />);
    expect(screen.getByTestId('host-key-dialog-address')).toHaveTextContent(/^192\.0\.2\.15$/);
  });
});

describe('HostKeyDialog: changed key', () => {
  it('blocks with a danger warning and the old and new keys, without any checkbox', () => {
    render(<HostKeyDialog prompt={changed} hostTitle="lab" onDecision={vi.fn()} />);
    const dialog = screen.getByTestId('host-key-dialog');
    expect(dialog).toHaveAttribute('data-kind', 'changed');
    expect(screen.getByTestId('host-key-dialog-warning')).toHaveTextContent(/lab/);
    expect(screen.getByTestId('host-key-dialog-old-key-type')).toHaveTextContent('ecdsa-sha2-nistp256');
    expect(screen.getByTestId('host-key-dialog-old-fingerprint')).toHaveTextContent('SHA256:savedTESTfingerprint');
    expect(screen.getByTestId('host-key-dialog-key-type')).toHaveTextContent('ssh-ed25519');
    expect(screen.getByTestId('host-key-dialog-fingerprint')).toHaveTextContent('SHA256:presentedTESTfingerprint');
    // No "don't ask again" (UI 10.2) and no way to accept or connect once.
    expect(within(dialog).queryByRole('checkbox')).not.toBeInTheDocument();
    expect(screen.queryByTestId('host-key-dialog-accept')).not.toBeInTheDocument();
    expect(screen.queryByTestId('host-key-dialog-once')).not.toBeInTheDocument();
  });

  it('focuses Cancel; only "Replace saved key" continues', async () => {
    const user = userEvent.setup();
    const onDecision = vi.fn();
    render(<HostKeyDialog prompt={changed} hostTitle="lab" onDecision={onDecision} />);
    expect(screen.getByTestId('host-key-dialog-cancel')).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.click(screen.getByTestId('host-key-dialog-replace'));
    expect(onDecision.mock.calls.map((call) => call[0])).toEqual(['cancel', 'replace']);
  });

  it('disables the buttons while a decision is sent', () => {
    render(<HostKeyDialog prompt={changed} hostTitle="lab" onDecision={vi.fn()} busy />);
    expect(screen.getByTestId('host-key-dialog-replace')).toBeDisabled();
    expect(screen.getByTestId('host-key-dialog-cancel')).toBeDisabled();
  });
});

describe('HostKeyRevokedWarning', () => {
  it('shows the revoked key with no button to continue', () => {
    render(<HostKeyRevokedWarning hostTitle="lab" hostKey={KEY} />);
    const warning = screen.getByTestId('host-key-revoked');
    expect(warning).toHaveAttribute('role', 'alert');
    expect(screen.getByTestId('host-key-revoked-key-type')).toHaveTextContent('ssh-ed25519');
    expect(screen.getByTestId('host-key-revoked-fingerprint')).toHaveTextContent('SHA256:presentedTESTfingerprint');
    expect(within(warning).queryAllByRole('button')).toEqual([]);
  });
});
