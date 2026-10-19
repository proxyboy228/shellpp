// A prompt of the connection screen (UI 4.8; docs/security.md 11.2):
// username, password, key passphrase or one keyboard-interactive question.
// Continue (Enter) sends `{kind: 'text', value, save}`, Cancel sends
// `{kind: 'cancel'}`. The typed value leaves component state right after it
// is sent (stage 2 plan, 6.7). Server texts (keyboard-interactive name,
// instruction and question) are plain text in a frame labeled with the host.
// The master password is never asked here.
//
// Owner: S7.

import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { Button, Checkbox, PasswordInput, TextInput } from '../../components';
import { sessionPromptReply, type PromptReply, type Uuid } from '../../ipc';
import { t } from '../../i18n';
import type { TextPrompt } from './prompts';
import styles from './connect.module.css';

export interface SessionPromptFormProps {
  sessionId: Uuid;
  prompt: TextPrompt;
  /** Host title, labels the frame of the server texts. */
  hostTitle: string;
  /** The pane is active and visible (default true): the input takes focus then, and when it becomes so. */
  focused?: boolean;
}

function labelOf(prompt: TextPrompt): string {
  switch (prompt.kind) {
    case 'username':
      return t('prompt.username.label');
    case 'password':
      return prompt.username !== undefined && prompt.username !== ''
        ? t('prompt.password.label_user', { username: prompt.username })
        : t('prompt.password.label');
    case 'passphrase':
      return prompt.key_label !== undefined && prompt.key_label !== ''
        ? t('prompt.passphrase.label_key', { key: prompt.key_label })
        : t('prompt.passphrase.label');
    case 'keyboard_interactive':
      return prompt.prompt;
  }
}

function errorOf(prompt: TextPrompt): string | undefined {
  switch (prompt.kind) {
    case 'password':
      return prompt.error ? t('prompt.password.error') : undefined;
    case 'passphrase':
      return prompt.error ? t('prompt.passphrase.error') : undefined;
    case 'keyboard_interactive':
      return prompt.error ? t('prompt.keyboard_interactive.error') : undefined;
    default:
      return undefined;
  }
}

export function SessionPromptForm({ sessionId, prompt, hostTitle, focused = true }: SessionPromptFormProps) {
  const [value, setValue] = useState('');
  const [visible, setVisible] = useState(false);
  const [save, setSave] = useState(false);
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const labelId = useId();
  const inputId = useId();
  const frameId = useId();

  useEffect(() => {
    if (focused) {
      inputRef.current?.focus();
    }
  }, [focused]);

  const canSave = (prompt.kind === 'password' || prompt.kind === 'passphrase') && prompt.can_save;
  const masked = prompt.kind === 'password' || prompt.kind === 'passphrase' || (prompt.kind === 'keyboard_interactive' && !prompt.echo);
  const error = errorOf(prompt);
  const label = labelOf(prompt);
  const serverQuestion = prompt.kind === 'keyboard_interactive';

  const send = (reply: PromptReply) => {
    setSending(true);
    // The typed value is dropped from state as soon as it is sent.
    setValue('');
    setVisible(false);
    sessionPromptReply({ session_id: sessionId, prompt_id: prompt.prompt_id, reply })
      .catch(() => {
        // The prompt is gone (answered elsewhere, cancelled or the session
        // ended): the next `sessions:changed` shows the current state.
      })
      .finally(() => setSending(false));
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (sending || (prompt.kind === 'username' && value.trim() === '')) {
      return;
    }
    send(canSave ? { kind: 'text', value, save } : { kind: 'text', value });
  };

  const inputProps = {
    ref: inputRef,
    id: inputId,
    value,
    disabled: sending,
    error,
    autoComplete: 'off',
    spellCheck: false,
    'aria-labelledby': labelId,
    onChange: (event: { target: { value: string } }) => setValue(event.target.value),
    'data-testid': 'session-prompt-input',
  } as const;

  return (
    <form className={styles.promptForm} onSubmit={submit} data-kind={prompt.kind} data-testid="session-prompt" noValidate>
      {serverQuestion ? (
        <section className={styles.serverFrame} aria-labelledby={frameId} data-testid="session-prompt-server-text">
          <p id={frameId} className={styles.serverFrameLabel}>
            {t('prompt.keyboard_interactive.from_server', { host: hostTitle })}
          </p>
          {prompt.name !== '' ? <p className={styles.serverName}>{prompt.name}</p> : null}
          {prompt.instruction !== '' ? <p className={styles.serverInstruction}>{prompt.instruction}</p> : null}
          <label id={labelId} htmlFor={inputId} className={styles.serverQuestion} data-testid="session-prompt-label">
            {label}
          </label>
        </section>
      ) : (
        <label id={labelId} htmlFor={inputId} className={styles.promptLabel} data-testid="session-prompt-label">
          {label}
        </label>
      )}
      {serverQuestion && prompt.count > 1 ? (
        <p className={styles.promptProgress} data-testid="session-prompt-progress">
          {t('prompt.keyboard_interactive.progress', { index: prompt.index + 1, count: prompt.count })}
        </p>
      ) : null}
      {masked ? (
        <PasswordInput
          {...inputProps}
          visible={visible}
          onVisibleChange={setVisible}
          toggleLabel={t('prompt.show_value')}
        />
      ) : (
        <TextInput {...inputProps} />
      )}
      {canSave ? (
        <Checkbox
          checked={save}
          onCheckedChange={setSave}
          label={prompt.kind === 'password' ? t('prompt.save_password') : t('prompt.save_passphrase')}
          data-testid="session-prompt-save"
        />
      ) : null}
      <div className={styles.promptActions}>
        <Button type="submit" variant="primary" loading={sending} data-testid="session-prompt-continue">
          {t('prompt.continue')}
        </Button>
        <Button disabled={sending} onClick={() => send({ kind: 'cancel' })} data-testid="session-prompt-cancel">
          {t('common.cancel')}
        </Button>
      </div>
    </form>
  );
}
