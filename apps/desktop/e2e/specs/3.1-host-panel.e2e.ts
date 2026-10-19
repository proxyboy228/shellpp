// UI 3.1 Host panel blocks: Address with the neutral OS icon, General
// (label, parent group with search and create, tags, color, notes,
// backspace), SSH (port, username, password, inherited placeholders) and
// Show more. Stage 2: the Connect button pinned to the bottom (disabled for
// a draft), and the credentials "+" with the Key option: paste the key text
// or choose a file, or pick an existing key; an encrypted key asks for its
// passphrase ("Save passphrase" off by default); the linked key field shows
// label and type and can be unlinked. Vault, Share, the identity,
// certificate and FIDO2 options, the identity hint, startup snippet, host
// chaining, proxy, Mosh and Add Telnet belong to later stages (hidden).

import { Key } from 'webdriverio';
import {
  activateVaults,
  addTag,
  attrOfAll,
  clearAndType,
  closePanel,
  createGroup,
  createHost,
  createSshHost,
  el,
  expectAbsent,
  expectPresent,
  goToRoot,
  groupIds,
  hostCard,
  hostKey,
  isFocused,
  linkPastedKey,
  linkedKeyId,
  openFolder,
  openHost,
  openKeyDialog,
  openNewHostPanel,
  pickGroup,
  pressEscape,
  runCommand,
  setField,
  startFresh,
  submitKeyDialog,
  tabIds,
  textAt,
  tid,
  valueOf,
  waitFor,
  waitForAbsent,
  waitForNewTab,
  waitForRecordId,
  waitForSaved,
  waitForSelectedTab,
  waitForShell,
  waitForTabClosed,
} from '../helpers/index.ts';
import { generatedSecret, keygen, startSshd, stopAllSshd, type GeneratedKey, type Sshd } from '../support/sshd.ts';

const HIDDEN = [
  'host-panel-vault', // stage 8
  'host-panel-share', // stage 8
  'host-panel-identity-merge-hint', // stage 3
  'host-panel-startup-snippet', // stage 5
  'host-panel-host-chaining', // stage 5
  'host-panel-proxy', // stage 5
  'host-panel-mosh', // stage 10
  'host-panel-add-telnet', // stage 9
];

/** Credential types of the "+" menu that belong to stage 3. */
const HIDDEN_CREDENTIALS = [
  'host-panel-credentials-menu-identity',
  'host-panel-credentials-menu-certificate',
  'host-panel-credentials-menu-fido2',
];

const VISIBLE = [
  'host-panel-os-icon',
  'host-panel-address',
  'host-panel-label',
  'host-panel-group',
  'host-panel-tags-input',
  'host-panel-color',
  'host-panel-notes',
  'host-panel-backspace',
  'host-panel-ssh-port',
  'host-panel-ssh-username',
  'host-panel-ssh-password',
  'host-panel-show-more',
  'host-panel-connect',
];

const SHOW_MORE = [
  'host-panel-agent-forwarding',
  'host-panel-env',
  'host-panel-charset',
  'host-panel-keepalive',
  'host-panel-record-history',
  'host-panel-record-output',
  'host-panel-theme',
];

async function expandShowMore(): Promise<void> {
  const toggle = await waitFor('host-panel-show-more');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
    await toggle.click();
  }
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
}

function envRow(index: number, child: string): string {
  return `[data-testid="host-panel-env-row"][data-index="${index}"] [data-testid="${child}"]`;
}

describe('UI 3.1 Host panel', () => {
  let base: string;
  let hostId: string;
  const password = `pw-${Date.now()}`;

  before(async () => {
    await startFresh();
    base = await createGroup({ name: 'base', username: 'inherit-user', port: 2222 });
    await createHost({ address: 'shared-tags.example.com', tags: ['shared-tag'] });
  });

  it('shows the stage-1 blocks and puts the focus in Address for a new host', async () => {
    await openNewHostPanel();
    await browser.waitUntil(() => isFocused('host-panel-address'), { timeout: 5_000 });
    await expectPresent(VISIBLE);
    await expectAbsent(HIDDEN);
    // A draft cannot connect.
    await expect(el('host-panel-connect')).toBeDisabled();
    await expandShowMore();
    await expectPresent(SHOW_MORE);
    await expectAbsent(HIDDEN);
  });

  it('creates the host from a valid address', async () => {
    await el('host-panel-address').click();
    await el('host-panel-address').addValue('panel.example.com');
    hostId = await waitForRecordId('host-panel');
    await waitForSaved('host-panel');
    await expect(el('host-panel-os-icon')).toBeDisplayed();
    // Neutral until a connection detects the OS.
    await expect(el('host-panel-os-icon')).not.toHaveAttribute('data-os-family');
    // A saved host with a valid address can connect (not done here: example.com).
    await expect(el('host-panel-connect')).toBeEnabled();
    await expectPresent(['host-panel-ssh-credentials-add']);
    await expectAbsent(['host-panel-ssh-linked-key']);
  });

  it('saves the label and shows it as the card title', async () => {
    await setField('host-panel-label', 'Panel host');
    await waitForSaved('host-panel');
    await browser.waitUntil(async () => (await hostCard(hostId).$('[data-testid="host-card-title"]').getText()) === 'Panel host');
  });

  it('picks the parent group with search', async () => {
    await el('host-panel-group').click();
    await waitFor('host-panel-group-popover');
    await el('host-panel-group-search').setValue('bas');
    await browser.waitUntil(async () => (await attrOfAll('host-panel-group-option', 'data-group-id')).includes(base));
    await el('host-panel-group-option', `[data-group-id="${base}"]`).click();
    await waitForAbsent('host-panel-group-popover');
    await waitForSaved('host-panel');
    expect(await el('host-panel-group').getText()).toContain('base');
  });

  it('creates a parent group on the fly in the current folder', async () => {
    const before = await groupIds();
    await el('host-panel-group').click();
    await waitFor('host-panel-group-popover');
    await el('host-panel-group-search').setValue('fresh-group');
    await el('host-panel-group-create').click();
    await waitForAbsent('host-panel-group-popover');
    await waitForSaved('host-panel');
    expect(await el('host-panel-group').getText()).toContain('fresh-group');
    await browser.waitUntil(async () => (await groupIds()).length === before.length + 1, {
      timeoutMsg: 'the new group does not appear at the top level',
    });
    await pickGroup('host-panel-group', base);
    await waitForSaved('host-panel');
    // The host now lives in "base": continue inside that folder.
    await closePanel('host-panel');
    await openFolder(base);
    await openHost(hostId);
  });

  it('adds tags with Enter and comma, autocompletes and removes chips', async () => {
    await addTag('alpha');
    const input = el('host-panel-tags-input');
    await input.addValue('beta,');
    await waitFor('host-panel-tag', '[data-tag="beta"]');
    await input.addValue('shar');
    const suggestion = await waitFor('host-panel-tags-suggestion', '[data-tag="shared-tag"]');
    await suggestion.click();
    await waitFor('host-panel-tag', '[data-tag="shared-tag"]');
    await el('host-panel-tag', '[data-tag="beta"]').$('[data-testid="host-panel-tag-remove"]').click();
    await waitForAbsent('host-panel-tag', '[data-tag="beta"]');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await openHost(hostId);
    expect(await attrOfAll('host-panel-tag', 'data-tag')).toEqual(['alpha', 'shared-tag']);
  });

  it('keeps the chosen color (graphite by default)', async () => {
    await expect(el('host-panel-color-graphite')).toHaveAttribute('aria-pressed', 'true');
    await el('host-panel-color-ocean').click();
    await expect(el('host-panel-color-ocean')).toHaveAttribute('aria-pressed', 'true');
    await expect(el('host-panel-color-graphite')).toHaveAttribute('aria-pressed', 'false');
    await waitForSaved('host-panel');
    await expect(hostCard(hostId)).toHaveAttribute('data-color', 'ocean');
  });

  it('keeps multi-line notes', async () => {
    const notes = el('host-panel-notes');
    await notes.click();
    await notes.addValue('line one');
    await browser.keys(Key.Enter);
    await notes.addValue('line two');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await openHost(hostId);
    expect((await valueOf('host-panel-notes')).replace(/\r\n/g, '\n')).toBe('line one\nline two');
  });

  it('stores the Backspace setting', async () => {
    const select = el('host-panel-backspace');
    expect(await select.getValue()).toBe('default');
    await select.selectByAttribute('value', 'ctrl_h');
    await waitForSaved('host-panel');
    await closePanel('host-panel');
    await openHost(hostId);
    expect(await valueOf('host-panel-backspace')).toBe('ctrl_h');
  });

  it('stores port and username, masks the password and reveals the saved one', async () => {
    await setField('host-panel-ssh-port', '2200');
    await setField('host-panel-ssh-username', 'panel-user');
    await setField('host-panel-ssh-password', password);
    await expect(el('host-panel-ssh-password')).toHaveAttribute('type', 'password');
    await waitForSaved('host-panel');
    await closePanel('host-panel');

    await openHost(hostId);
    expect(await valueOf('host-panel-ssh-port')).toBe('2200');
    expect(await valueOf('host-panel-ssh-username')).toBe('panel-user');
    // The webview gets the secret only on request (secret_reveal).
    expect(await valueOf('host-panel-ssh-password')).toBe('');
    await el('host-panel-ssh-password-toggle').click();
    await browser.waitUntil(async () => (await valueOf('host-panel-ssh-password')) === password, {
      timeout: 10_000,
      timeoutMsg: 'the saved password was not revealed',
    });
    await expect(el('host-panel-ssh-password')).toHaveAttribute('type', 'text');
    await el('host-panel-ssh-password-toggle').click();
    await expect(el('host-panel-ssh-password')).toHaveAttribute('type', 'password');
    await closePanel('host-panel');
  });

  it('shows inherited port and username as "from group" placeholders', async () => {
    const inheriting = await createHost({ address: 'inherits.example.com', groupId: base, close: false });
    const port = (await el('host-panel-ssh-port').getAttribute('placeholder')) ?? '';
    const user = (await el('host-panel-ssh-username').getAttribute('placeholder')) ?? '';
    expect(port).toContain('2222');
    expect(port).toContain('base');
    expect(user).toContain('inherit-user');
    expect(user).toContain('base');
    expect(await valueOf('host-panel-ssh-port')).toBe('');
    await closePanel('host-panel');
    expect(inheriting).not.toBe('');

    await goToRoot();
    const topLevel = await createHost({ address: 'top-level.example.com', close: false });
    expect(await el('host-panel-ssh-port').getAttribute('placeholder')).toBe('22');
    await closePanel('host-panel');
    expect(topLevel).not.toBe('');
    await openFolder(base);
  });

  it('stores the Show more settings', async () => {
    await openHost(hostId);
    await expandShowMore();

    const agent = el('host-panel-agent-forwarding');
    await expect(agent).toHaveAttribute('aria-checked', 'false');
    await agent.click();
    await expect(agent).toHaveAttribute('aria-checked', 'true');

    await el('host-panel-env-add').click();
    await waitFor('host-panel-env-row', '[data-index="0"]');
    await $(envRow(0, 'host-panel-env-name')).setValue('FOO');
    await $(envRow(0, 'host-panel-env-value')).setValue('bar');
    await el('host-panel-env-add').click();
    await waitFor('host-panel-env-row', '[data-index="1"]');
    await $(envRow(1, 'host-panel-env-name')).setValue('FOO');
    await $(envRow(1, 'host-panel-env-name-error')).waitForExist();
    await $(envRow(1, 'host-panel-env-name')).setValue('BAZ');
    await $(envRow(1, 'host-panel-env-value')).setValue('qux');
    await $(envRow(1, 'host-panel-env-name-error')).waitForExist({ reverse: true });

    const charset = el('host-panel-charset');
    expect(await charset.getValue()).toBe('utf-8');
    await charset.selectByAttribute('value', 'koi8-r');

    await clearAndType('host-panel-keepalive', '5000');
    await waitFor('host-panel-keepalive-error');
    expect((await el('host-panel-keepalive').getAttribute('placeholder')) ?? '').toContain('30');
    await clearAndType('host-panel-keepalive', '60');
    await waitForAbsent('host-panel-keepalive-error');

    await el('host-panel-record-history').selectByAttribute('value', 'on');
    await el('host-panel-record-output').selectByAttribute('value', 'off');

    await el('host-panel-theme').click();
    await waitFor('host-panel-theme-picker');
    await expectPresent(['host-panel-theme-option-default', 'host-panel-theme-option-graphite', 'host-panel-theme-option-paper']);
    await el('host-panel-theme-option-paper').click();
    await waitForAbsent('host-panel-theme-picker');
    expect(await el('host-panel-theme').getText()).toContain('Paper');
    await waitForSaved('host-panel');
    await closePanel('host-panel');

    await openHost(hostId);
    await expandShowMore();
    await expect(el('host-panel-agent-forwarding')).toHaveAttribute('aria-checked', 'true');
    expect(await browser.execute(() => {
      const values = (name: string) =>
        Array.from(document.querySelectorAll(`[data-testid="${name}"]`)).map((node) => (node as HTMLInputElement).value);
      return { names: values('host-panel-env-name'), values: values('host-panel-env-value') };
    })).toEqual({ names: ['FOO', 'BAZ'], values: ['bar', 'qux'] });
    expect(await valueOf('host-panel-charset')).toBe('koi8-r');
    expect(await valueOf('host-panel-keepalive')).toBe('60');
    expect(await valueOf('host-panel-record-history')).toBe('on');
    expect(await valueOf('host-panel-record-output')).toBe('off');
    expect(await el('host-panel-theme').getText()).toContain('Paper');
    await expectAbsent(HIDDEN);
    await closePanel('host-panel');
  });

  describe('Connect and "+ -> Key"', () => {
    let sshd: Sshd;
    let encrypted: GeneratedKey;
    let keyHost: string;
    let pastedKey: string;

    /** Connects with the pinned Connect button of the open panel and waits for the shell. */
    async function connectFromPanel(user: string, acceptKey: boolean): Promise<string> {
      const before = await tabIds();
      await el('host-panel-connect').click();
      const tab = await waitForNewTab(before);
      await waitForSelectedTab(tab);
      if (acceptKey) {
        await hostKey('accept');
      }
      await waitForShell(user);
      return tab;
    }

    /** Selects a file in the dialog's file input (`key-add-file`). */
    async function chooseKeyFile(file: string): Promise<void> {
      const input = el('key-add-file');
      await input.waitForExist();
      try {
        await input.setValue(file);
      } catch {
        // A visually hidden file input: make it interactable for WebDriver.
        await browser.execute((selector: string) => {
          const node = document.querySelector(selector) as HTMLElement | null;
          if (node) {
            node.hidden = false;
            node.style.display = 'block';
            node.style.visibility = 'visible';
            node.style.opacity = '1';
          }
        }, tid('key-add-file'));
        await input.setValue(file);
      }
    }

    before(async () => {
      encrypted = await keygen('ed25519', { passphrase: generatedSecret('Pass-'), comment: 'panel-key' });
      sshd = await startSshd({ users: [{ name: 'keyuser', publicKeys: [encrypted.publicKey] }] });
      await goToRoot();
      keyHost = await createSshHost(sshd, sshd.users[0], { label: 'Key panel host' });
    });

    after(async () => {
      await stopAllSshd();
    });

    it('offers only the Key type in the "+" menu', async () => {
      await openHost(keyHost);
      await el('host-panel-ssh-credentials-add').click();
      await waitFor('host-panel-credentials-menu');
      await expectPresent(['host-panel-credentials-menu-key']);
      await expectAbsent(HIDDEN_CREDENTIALS);
      await pressEscape();
      await waitForAbsent('host-panel-credentials-menu');
    });

    it('adds a pasted encrypted key with its saved passphrase and links it', async () => {
      pastedKey = await linkPastedKey({
        privateKey: encrypted.privateKey,
        passphrase: encrypted.passphrase,
        savePassphrase: true,
        label: 'Panel key',
      });
      await waitForSaved('host-panel', { changed: false });
      expect(await textAt(`${tid('host-panel-ssh-linked-key')} ${tid('host-panel-ssh-linked-key-label')}`)).toBe('Panel key');
      expect(await textAt(`${tid('host-panel-ssh-linked-key')} ${tid('host-panel-ssh-linked-key-type')}`)).toContain('ed25519');
      await expectAbsent(['host-panel-ssh-linked-key-missing', 'host-panel-identity-merge-hint']);
      // No credential type is left to add in stage 2.
      await expectAbsent(['host-panel-ssh-credentials-add']);
    });

    it('connects with the linked key (the passphrase is saved: no question)', async () => {
      const tab = await connectFromPanel('keyuser', true);
      await runCommand('exit');
      await waitForTabClosed(tab);
    });

    it('unlinks the key and links it again from the existing keys', async () => {
      await activateVaults();
      await openHost(keyHost);
      await el('host-panel-ssh-linked-key-unlink').click();
      await waitForAbsent('host-panel-ssh-linked-key');
      await waitFor('host-panel-ssh-credentials-add');
      await waitForSaved('host-panel');

      await openKeyDialog();
      const existing = el('key-add-existing-item', `[data-record-id="${pastedKey}"]`);
      await existing.waitForExist();
      await existing.click();
      await waitForAbsent('key-add-dialog');
      expect(await linkedKeyId()).toBe(pastedKey);
      await waitForSaved('host-panel');
      await el('host-panel-ssh-linked-key-unlink').click();
      await waitForAbsent('host-panel-ssh-linked-key');
      await waitForSaved('host-panel');
    });

    it('adds a key from a file', async () => {
      const plain = await keygen('ed25519', { comment: 'file-key' });
      await openKeyDialog();
      await chooseKeyFile(plain.path);
      await clearAndType('key-add-label', 'File key');
      await submitKeyDialog({});
      const id = await linkedKeyId();
      expect(id).not.toBe(pastedKey);
      expect(await textAt(`${tid('host-panel-ssh-linked-key')} ${tid('host-panel-ssh-linked-key-label')}`)).toBe('File key');
      await waitForSaved('host-panel', { changed: false });
      await closePanel('host-panel');
    });
  });
});
