// "+ -> Key" (UI 3.1): existing keys, paste or file, field errors, the
// passphrase step with the store-or-ask choice. Uses the mock backend's fake
// key parser (stage 2 plan 6.6); passphrases are generated at run time.

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setBackend } from '../../ipc';
import { createMockBackend, type MockBackend } from '../../ipc/mock';
import { KeyAddDialog, MAX_KEY_FILE_BYTES } from './KeyAddDialog';

let backend: MockBackend;
let vaultId: string;

beforeEach(async () => {
  backend = createMockBackend({ initialState: 'unlocked', password: `pw-${crypto.randomUUID()}` });
  setBackend(backend);
  vaultId = (await backend.invoke('state_snapshot', undefined)).vaults[0]?.vault_id ?? '';
});

afterEach(() => {
  setBackend(null);
});

function keyText(options: { passphrase?: string; comment?: string; algorithm?: string } = {}): string {
  return [
    '-----BEGIN OPENSSH PRIVATE KEY-----',
    crypto.randomUUID(),
    ...(options.algorithm === undefined ? [] : [`mock-algorithm:${options.algorithm}`]),
    ...(options.comment === undefined ? [] : [`mock-comment:${options.comment}`]),
    ...(options.passphrase === undefined ? [] : [`mock-encrypted:${options.passphrase}`]),
    '-----END OPENSSH PRIVATE KEY-----',
  ].join('\n');
}

function renderDialog() {
  const onLinked = vi.fn();
  const onClose = vi.fn();
  render(<KeyAddDialog vaultId={vaultId} onLinked={onLinked} onClose={onClose} />);
  return { onLinked, onClose };
}

/** Pastes text into the key field (typing a long key is slow and needs escapes). */
function paste(text: string) {
  fireEvent.change(screen.getByTestId('key-add-paste'), { target: { value: text } });
}

describe('KeyAddDialog', () => {
  it('imports a pasted key with a label and links it', async () => {
    const user = userEvent.setup();
    const { onLinked, onClose } = renderDialog();
    expect(screen.getByTestId('key-add-paste')).toHaveFocus();
    expect(screen.queryByTestId('key-add-existing')).not.toBeInTheDocument();
    expect(screen.queryByTestId('key-add-passphrase')).not.toBeInTheDocument();
    paste(keyText({ comment: 'laptop' }));
    await user.type(screen.getByTestId('key-add-label'), 'Work key');
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
    expect(onLinked.mock.calls[0]?.[0]).toMatchObject({ label: 'Work key', algorithm: 'ssh-ed25519', encrypted: false });
    expect(onClose).toHaveBeenCalled();
  });

  it('maps key errors to the key field', async () => {
    const user = userEvent.setup();
    const { onLinked } = renderDialog();
    await user.click(screen.getByTestId('key-add-submit'));
    expect(screen.getByTestId('key-add-paste-error')).toHaveTextContent('Paste a private key');
    paste('-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----');
    expect(screen.queryByTestId('key-add-paste-error')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(screen.getByTestId('key-add-paste-error')).toHaveTextContent('not an OpenSSH private key'));
    expect(screen.getByTestId('key-add-paste')).toHaveAttribute('aria-invalid', 'true');
    paste(keyText({ algorithm: 'ssh-dss' }));
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(screen.getByTestId('key-add-paste-error')).toHaveTextContent('not supported'));
    expect(onLinked).not.toHaveBeenCalled();
  });

  it('maps a label error to the label field', async () => {
    const user = userEvent.setup();
    renderDialog();
    paste(keyText());
    fireEvent.change(screen.getByTestId('key-add-label'), { target: { value: 'L'.repeat(300) } });
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(screen.getByTestId('key-add-label-error')).toHaveTextContent('too long'));
  });

  it('asks for the passphrase of an encrypted key; a wrong one is an error; "ask every time" by default', async () => {
    const user = userEvent.setup();
    const passphrase = `pp-${crypto.randomUUID()}`;
    const { onLinked } = renderDialog();
    paste(keyText({ passphrase }));
    await user.click(screen.getByTestId('key-add-submit'));
    const field = await screen.findByTestId('key-add-passphrase');
    await waitFor(() => expect(field).toHaveFocus());
    expect(screen.queryByTestId('key-add-passphrase-error')).not.toBeInTheDocument();
    expect(screen.getByTestId('key-add-save-passphrase')).not.toBeChecked();

    await user.type(field, `${passphrase}-wrong{Enter}`);
    await waitFor(() => expect(screen.getByTestId('key-add-passphrase-error')).toHaveTextContent('Wrong passphrase'));
    expect(field).toHaveValue('');
    expect(onLinked).not.toHaveBeenCalled();

    await user.type(field, `${passphrase}{Enter}`);
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
    expect(onLinked.mock.calls[0]?.[0]).toMatchObject({ encrypted: true, passphrase_saved: false });
  });

  it('saves the passphrase when chosen', async () => {
    const user = userEvent.setup();
    const passphrase = `pp-${crypto.randomUUID()}`;
    const { onLinked } = renderDialog();
    paste(keyText({ passphrase }));
    await user.click(screen.getByTestId('key-add-submit'));
    await user.type(await screen.findByTestId('key-add-passphrase'), passphrase);
    await user.click(screen.getByTestId('key-add-save-passphrase'));
    await user.click(screen.getByTestId('key-add-submit'));
    await waitFor(() => expect(onLinked).toHaveBeenCalledTimes(1));
    expect(onLinked.mock.calls[0]?.[0]).toMatchObject({ encrypted: true, passphrase_saved: true });
  });

  it('lists saved keys and links one with a click', async () => {
    const user = userEvent.setup();
    const imported = await backend.invoke('key_import', { vault_id: vaultId, private_key: keyText({ comment: 'ci key' }) });
    const { onLinked, onClose } = renderDialog();
    const item = await screen.findByTestId('key-add-existing-item');
    expect(item).toHaveAttribute('data-record-id', imported.key.record_id);
    expect(item).toHaveTextContent('ci key');
    expect(item).toHaveTextContent('ssh-ed25519');
    await user.click(item);
    expect(onLinked).toHaveBeenCalledWith(imported.key);
    expect(onClose).toHaveBeenCalled();
  });

  it('reads a chosen key file into the key field and refuses large files', async () => {
    const user = userEvent.setup();
    renderDialog();
    const text = keyText({ comment: 'from file' });
    await user.upload(screen.getByTestId('key-add-file'), new File([text], 'id_test', { type: 'text/plain' }));
    await waitFor(() => expect(screen.getByTestId('key-add-paste')).toHaveValue(text));

    const big = new File(['A'.repeat(MAX_KEY_FILE_BYTES + 1)], 'big', { type: 'text/plain' });
    await act(async () => {
      await user.upload(screen.getByTestId('key-add-file'), big);
    });
    expect(screen.getByTestId('key-add-paste-error')).toHaveTextContent('larger than 64 KiB');
  });

  it('refuses copying the private key text', () => {
    renderDialog();
    paste(keyText());
    const event = new Event('copy', { bubbles: true, cancelable: true });
    screen.getByTestId('key-add-paste').dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(screen.getByTestId('key-add-paste')).toHaveAttribute('data-sensitive');
  });

  it('closes with Cancel', async () => {
    const user = userEvent.setup();
    const { onClose } = renderDialog();
    await user.click(screen.getByTestId('key-add-cancel'));
    expect(onClose).toHaveBeenCalled();
  });
});
