// Host panel (UI 3.1-3.3) in the details panel: create and edit a host with
// autosave. Stage 1 shows the Address, General and SSH blocks and the "Show
// more" settings; later-stage blocks are hidden (src/stage.ts).

import { ChevronDown, ChevronUp, Copy, Server, Trash2 } from 'lucide-react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { DetailsPanel } from '../../app/DetailsPanel/DetailsPanel';
import type { ContextMenuEntry } from '../../app/ContextMenu/ContextMenu';
import { Button, PasswordInput, Select, TextInput, Toggle } from '../../components';
import { recordSave, secretReveal, toIpcError } from '../../ipc';
import type { GroupView, HostData, HostView, SettingsValues, ThemeRef, Uuid } from '../../ipc';
import { t } from '../../i18n';
import { showToast } from '../../app/toast';
import { reasonFor } from './form';
import { BACKSPACE_VALUES, HOST_FIELDS, RECORD_MODES, type HostForm } from './fields';
import { inheritedPlaceholder, resolveInherited } from './inheritance';
import { usePanelRootAttributes } from './panelAttributes';
import { useRecordForm } from './useRecordForm';
import { CHARSETS, parseSecret } from './validators';
import { ColorField } from './fields/ColorField';
import { EnvVarsField } from './fields/EnvVarsField';
import { GroupPicker } from './fields/GroupPicker';
import { TagsField } from './fields/TagsField';
import { TextAreaField } from './fields/TextAreaField';
import { ThemePicker } from './fields/ThemePicker';
import styles from './panels.module.css';

export const HOST_PANEL_TEST_ID = 'host-panel';
const TID = HOST_PANEL_TEST_ID;

export interface HostPanelProps {
  vaultId: Uuid;
  /** `null`: a new host (a draft until the address is valid). */
  recordId: Uuid | null;
  /** The host as the screen's lists show it. */
  view: HostView | undefined;
  /** Group of a new host (the open folder). */
  draftGroupId: Uuid | null;
  groups: readonly GroupView[];
  groupsById: ReadonlyMap<Uuid, GroupView>;
  /** Tags of all hosts, for autocomplete. */
  allTags: readonly string[];
  settings: SettingsValues;
  showMore: boolean;
  onShowMoreChange: (expanded: boolean) => void;
  /** Each new value moves focus to Address (creation, the E key). */
  focusNonce: number;
  /** The draft was created. */
  onCreated: (view: HostView) => void;
  onClose: () => void;
  onDuplicate: (recordId: Uuid) => void;
  onRemove: (recordId: Uuid) => void;
  /** Creates a group in the open folder; resolves with its id. */
  onCreateGroup: (name: string) => Promise<Uuid | null>;
  /** The host was removed elsewhere. */
  onMissing: () => void;
}

function settingNumber(settings: SettingsValues, key: string): number | undefined {
  const value = settings[key];
  return typeof value === 'number' ? value : undefined;
}

function settingTheme(settings: SettingsValues): ThemeRef | null {
  const value = settings['terminal.default_theme'];
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    if (typeof value.builtin === 'string') {
      return { builtin: value.builtin };
    }
    if (typeof value.record_id === 'string') {
      return { record_id: value.record_id };
    }
  }
  return null;
}

export function HostPanel({
  vaultId,
  recordId: initialRecordId,
  view,
  draftGroupId,
  groups,
  groupsById,
  allTags,
  settings,
  showMore,
  onShowMoreChange,
  focusNonce,
  onCreated,
  onClose,
  onDuplicate,
  onRemove,
  onCreateGroup,
  onMissing,
}: HostPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const addressRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);

  // Password (a secret path): never part of `data`. `secretChange`:
  // undefined = keep, string = set, null = clear.
  const [password, setPassword] = useState('');
  const [secretChange, setSecretChange] = useState<string | null | undefined>(undefined);
  const [typed, setTyped] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const [visible, setVisible] = useState(false);
  const passwordCheck = parseSecret(password, 4096);
  const secretPending = secretChange !== undefined && passwordCheck.ok;
  const secretRef = useRef<string | null | undefined>(undefined);
  useLayoutEffect(() => {
    secretRef.current = secretPending ? secretChange : undefined;
  });

  const record = useRecordForm<HostForm, HostView>({
    table: HOST_FIELDS,
    requiredKey: 'address',
    recordId: initialRecordId,
    view,
    draftData: draftGroupId === null ? {} : { group_id: draftGroupId },
    extraDirty: secretPending,
    persist: async (recordId, data) => {
      const change = secretRef.current;
      const saved = await recordSave({
        vault_id: vaultId,
        record_id: recordId,
        type: 'host',
        data: data as HostData,
        ...(change === undefined ? {} : { secrets: { 'ssh.password': change } }),
      });
      if (saved.type !== 'host') {
        throw new Error('record_save returned another type');
      }
      if (change !== undefined) {
        setSecretChange((current) => (current === change ? undefined : current));
        // New secret values are not kept in the webview after saving
        // (security.md 9.5): clear the field unless the user is still typing.
        if (document.activeElement !== passwordRef.current) {
          setPassword((current) => (current === change ? '' : current));
          setTyped(false);
          setVisible(false);
        }
      }
      return saved;
    },
    onSaved: (saved, created) => {
      if (created) {
        onCreated(saved);
      }
    },
    onMissing,
  });
  const { form, errors, update, recordId, base } = record;
  const isDraft = recordId === null;

  usePanelRootAttributes(containerRef, TID, {
    'data-record-id': recordId ?? undefined,
    'data-save-state': record.saveState,
  });

  useEffect(() => {
    if (focusNonce > 0) {
      addressRef.current?.focus();
    }
  }, [focusNonce]);

  const message = (path: string): string | undefined => {
    const reason = reasonFor(errors, path);
    return reason === undefined ? undefined : t(`validation.${reason}`);
  };

  const inherited = useMemo(() => resolveInherited(form.group_id, groupsById), [form.group_id, groupsById]);
  const keepaliveGlobal = settingNumber(settings, 'terminal.keepalive_s');
  const passwordSaved = base?.secrets_set.includes('ssh.password') ?? false;

  const handlePasswordChange = (value: string) => {
    setPassword(value);
    setTyped(true);
    if (value === '') {
      setSecretChange(passwordSaved ? null : undefined);
    } else {
      setSecretChange(value);
    }
    record.touch();
  };

  const handlePasswordVisible = (next: boolean) => {
    if (next && passwordSaved && !revealed && !typed && password === '' && recordId !== null) {
      secretReveal({ vault_id: vaultId, record_id: recordId, path: 'ssh.password' }).then(
        (secret) => {
          setPassword(secret.value);
          setRevealed(true);
          setVisible(true);
        },
        (reason: unknown) => {
          showToast({ message: t(`errors.${toIpcError(reason).code}`), level: 'error', testId: 'panels-reveal-error-toast' });
        },
      );
      return;
    }
    setVisible(next);
  };

  const handlePasswordBlur = () => {
    if (typed && secretChange === undefined) {
      setPassword('');
      setTyped(false);
      setVisible(false);
    }
  };

  const menuItems: ContextMenuEntry[] =
    recordId === null
      ? []
      : [
          // `host-connect` (stage 2) and `host-telnet` (stage 9) items are hidden.
          {
            id: 'duplicate',
            label: t('host_panel.menu.duplicate'),
            icon: Copy,
            onSelect: () => {
              record.flush();
              onDuplicate(recordId);
            },
            'data-testid': `${TID}-menu-duplicate`,
          },
          { type: 'separator', id: 'separator' },
          {
            id: 'remove',
            label: t('host_panel.menu.remove'),
            icon: Trash2,
            danger: true,
            onSelect: () => onRemove(recordId),
            'data-testid': `${TID}-menu-remove`,
          },
        ];

  let passwordPlaceholder = '';
  if (passwordSaved && password === '' && secretChange === undefined) {
    passwordPlaceholder = t('host_panel.password_saved');
  }

  return (
    <div ref={containerRef} className={styles.panelSlot}>
      <DetailsPanel
        title={isDraft ? t('host_panel.title_new') : t('host_panel.title')}
        menuItems={menuItems}
        onCollapse={onClose}
        data-testid={TID}
        data-record-id={recordId ?? undefined}
        data-save-state={record.saveState}
      >
        <div className={styles.form}>
          {/* `panel-vault-field` (stage 8) is hidden. */}
          <div className={styles.addressRow}>
            <span className={styles.osTile} data-color={form.color} data-testid={`${TID}-os-icon`} aria-hidden="true">
              <Server size={20} strokeWidth={1.75} aria-hidden="true" focusable="false" />
            </span>
            <TextInput
              ref={addressRef}
              className={styles.grow}
              label={t('host_panel.address')}
              placeholder={t('host_panel.address_placeholder')}
              value={form.address}
              autoComplete="off"
              spellCheck={false}
              error={message('address')}
              onChange={(event) => update('address', event.target.value)}
              data-testid={`${TID}-address`}
            />
          </div>

          <section className={styles.block} aria-labelledby={`${TID}-general-title`}>
            <h3 id={`${TID}-general-title`} className={styles.blockTitle}>
              {t('host_panel.general')}
            </h3>
            <TextInput
              label={t('host_panel.label')}
              value={form.label}
              autoComplete="off"
              error={message('label')}
              onChange={(event) => update('label', event.target.value)}
              data-testid={`${TID}-label`}
            />
            <GroupPicker
              label={t('host_panel.group')}
              value={form.group_id}
              onChange={(groupId) => update('group_id', groupId)}
              groups={groups}
              groupsById={groupsById}
              onCreate={onCreateGroup}
              data-testid={`${TID}-group`}
              createTestId={`${TID}-group-create`}
            />
            <TagsField
              tags={form.tags}
              onChange={(tags) => update('tags', tags)}
              suggestions={allTags}
              error={reasonFor(errors, 'tags')}
              testIdPrefix={TID}
            />
            <ColorField value={form.color} onChange={(color) => update('color', color)} data-testid={`${TID}-color`} />
            <TextAreaField
              label={t('host_panel.notes')}
              value={form.notes}
              error={message('notes')}
              onChange={(notes) => update('notes', notes)}
              data-testid={`${TID}-notes`}
            />
            <Select
              label={t('host_panel.backspace.label')}
              value={form.backspace}
              options={BACKSPACE_VALUES.map((value) => ({ value, label: t(`host_panel.backspace.${value}`) }))}
              onValueChange={(value) => update('backspace', value)}
              data-testid={`${TID}-backspace`}
            />
          </section>

          {/* `host-share` (stage 8) is hidden. */}

          <section className={styles.block} aria-labelledby={`${TID}-ssh-title`}>
            <div className={styles.sshHeader}>
              <h3 id={`${TID}-ssh-title`} className={styles.blockTitle}>
                {t('host_panel.ssh')}
              </h3>
              <TextInput
                className={styles.portField}
                aria-label={t('host_panel.port')}
                inputMode="numeric"
                autoComplete="off"
                placeholder={inheritedPlaceholder(inherited.port, '22')}
                value={form.port}
                error={message('ssh.port')}
                onChange={(event) => update('port', event.target.value)}
                data-testid={`${TID}-ssh-port`}
              />
            </div>
            {/* `host-credentials-identity` (stage 3): identity merge hint is hidden. */}
            <TextInput
              label={t('host_panel.username')}
              autoComplete="off"
              spellCheck={false}
              placeholder={inheritedPlaceholder(inherited.username)}
              value={form.username}
              error={message('ssh.username')}
              onChange={(event) => update('username', event.target.value)}
              data-testid={`${TID}-ssh-username`}
            />
            <PasswordInput
              ref={passwordRef}
              label={t('host_panel.password')}
              toggleLabel={t('host_panel.password_show')}
              placeholder={passwordPlaceholder}
              value={password}
              visible={visible}
              onVisibleChange={handlePasswordVisible}
              error={passwordCheck.ok ? undefined : t(`validation.${passwordCheck.reason}`)}
              onChange={(event) => handlePasswordChange(event.target.value)}
              onBlur={handlePasswordBlur}
              data-testid={`${TID}-ssh-password`}
            />
            {/* `host-credentials-key` (stage 2): "+" credentials and the linked key are hidden. */}
            <div>
              <Button
                variant="text"
                icon={showMore ? <ChevronUp /> : <ChevronDown />}
                aria-expanded={showMore}
                onClick={() => onShowMoreChange(!showMore)}
                data-testid={`${TID}-show-more`}
              >
                {showMore ? t('host_panel.show_less') : t('host_panel.show_more')}
              </Button>
            </div>
            {showMore ? (
              <div className={styles.more}>
                <Toggle
                  label={t('host_panel.agent_forwarding')}
                  checked={form.agent_forwarding}
                  onCheckedChange={(checked) => update('agent_forwarding', checked)}
                  data-testid={`${TID}-agent-forwarding`}
                />
                {/* `host-startup-snippet`, `host-chaining`, `host-proxy` (stage 5) are hidden. */}
                <EnvVarsField rows={form.env} onChange={(rows) => update('env', rows)} errors={errors} testIdPrefix={TID} />
                <Select
                  label={t('host_panel.charset')}
                  value={form.charset}
                  options={[
                    ...CHARSETS.map((value) => ({ value, label: value.toUpperCase() })),
                    ...((CHARSETS as readonly string[]).includes(form.charset)
                      ? []
                      : [{ value: form.charset, label: form.charset.toUpperCase() }]),
                  ]}
                  error={message('charset')}
                  onValueChange={(value) => update('charset', value)}
                  data-testid={`${TID}-charset`}
                />
                <TextInput
                  label={t('host_panel.keepalive')}
                  inputMode="numeric"
                  autoComplete="off"
                  placeholder={
                    keepaliveGlobal === undefined ? '' : t('host_panel.keepalive_placeholder', { value: keepaliveGlobal })
                  }
                  value={form.keepalive}
                  error={message('ssh.keepalive_s')}
                  onChange={(event) => update('keepalive', event.target.value)}
                  data-testid={`${TID}-keepalive`}
                />
                <Select
                  label={t('host_panel.record_history')}
                  value={form.record_history}
                  options={RECORD_MODES.map((value) => ({ value, label: t(`host_panel.record_mode.${value}`) }))}
                  onValueChange={(value) => update('record_history', value)}
                  data-testid={`${TID}-record-history`}
                />
                <Select
                  label={t('host_panel.record_output')}
                  value={form.record_output}
                  options={RECORD_MODES.map((value) => ({ value, label: t(`host_panel.record_mode.${value}`) }))}
                  onValueChange={(value) => update('record_output', value)}
                  data-testid={`${TID}-record-output`}
                />
                {/* `host-mosh` (stage 10) is hidden. */}
                <ThemePicker
                  value={form.theme}
                  onChange={(theme) => update('theme', theme)}
                  defaultTheme={settingTheme(settings)}
                  data-testid={`${TID}-theme`}
                />
              </div>
            ) : null}
          </section>
          {/* `host-telnet` (stage 9) and the `host-connect` button (stage 2) are hidden. */}
        </div>
      </DetailsPanel>
    </div>
  );
}

export default HostPanel;
