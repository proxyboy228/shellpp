// "+ -> Key" (UI 3.1; stage 2 plan 6.3, decision 9): link an existing key,
// or add a new one by pasting the OpenSSH private key text or choosing a file
// (read here, at most 64 KiB). An encrypted key asks for its passphrase with
// the choice to save it in the encrypted storage or to enter it on every
// connection (UI 7.3). The key is stored as a `key` record (`key_import`)
// and the caller links it as `ssh.key_id`.
//
// The private key and the passphrase are secrets: they stay in this
// component only until they are sent and are dropped when it closes
// (docs/security.md 9.5); copy and cut are refused in the key field (9.4).
//
// Owner: S7.

import { FileUp, KeyRound } from 'lucide-react';
import { useEffect, useId, useRef, useState, type ChangeEvent, type ClipboardEvent, type FormEvent, type KeyboardEvent } from 'react';
import { Button, Checkbox, Modal, PasswordInput, TextInput } from '../../components';
import { keyImport, toIpcError, type KeyView, type Uuid, type ValidationReason } from '../../ipc';
import { t } from '../../i18n';
import { useKeys } from './useKeys';
import styles from './connect.module.css';

/** Largest key file read in the webview (decision 9). */
export const MAX_KEY_FILE_BYTES = 64 * 1024;

export interface KeyAddDialogProps {
  vaultId: Uuid;
  /** A key was chosen or imported: link it. */
  onLinked(key: KeyView): void;
  onClose(): void;
}

interface Errors {
  privateKey?: string;
  label?: string;
  passphrase?: string;
  general?: string;
}

function privateKeyMessage(reason: ValidationReason): string {
  switch (reason) {
    case 'too_long':
      return t('key_add.error.too_long');
    case 'not_allowed':
      return t('key_add.error.not_allowed');
    case 'required':
      return t('key_add.error.required');
    default:
      return t('key_add.error.invalid');
  }
}

/** Reads a chosen key file as text (Blob.text where available). */
async function readText(file: File): Promise<string> {
  if (typeof file.text === 'function') {
    return file.text();
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : '');
    reader.onerror = () => reject(new Error('read failed'));
    reader.readAsText(file);
  });
}

export function KeyAddDialog({ vaultId, onLinked, onClose }: KeyAddDialogProps) {
  const keys = useKeys();
  const [privateKey, setPrivateKey] = useState('');
  const [label, setLabel] = useState('');
  const [passphrase, setPassphrase] = useState('');
  const [passphraseVisible, setPassphraseVisible] = useState(false);
  const [needPassphrase, setNeedPassphrase] = useState(false);
  const [savePassphrase, setSavePassphrase] = useState(false);
  const [errors, setErrors] = useState<Errors>({});
  const [submitting, setSubmitting] = useState(false);
  const pasteRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const passphraseRef = useRef<HTMLInputElement>(null);
  const pasteId = useId();
  const fileId = useId();

  useEffect(() => {
    if (needPassphrase) {
      passphraseRef.current?.focus();
    }
  }, [needPassphrase]);

  const changePrivateKey = (value: string) => {
    setPrivateKey(value);
    setFileName(null);
    setErrors((current) => ({ ...current, privateKey: undefined, general: undefined }));
    // Another key: its passphrase step starts over.
    setNeedPassphrase(false);
    setPassphrase('');
  };

  const chooseFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.target;
    const file = input.files?.[0];
    input.value = '';
    if (file === undefined) {
      return;
    }
    if (file.size > MAX_KEY_FILE_BYTES) {
      setErrors((current) => ({ ...current, privateKey: t('key_add.error.file_too_large') }));
      return;
    }
    try {
      changePrivateKey(await readText(file));
      setFileName(file.name);
    } catch {
      setErrors((current) => ({ ...current, privateKey: t('key_add.error.file_unreadable') }));
    }
  };

  const link = (key: KeyView) => {
    setPrivateKey('');
    setPassphrase('');
    onLinked(key);
    onClose();
  };

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (submitting) {
      return;
    }
    if (privateKey.trim() === '') {
      setErrors({ privateKey: t('key_add.error.required') });
      pasteRef.current?.focus();
      return;
    }
    setSubmitting(true);
    setErrors({});
    try {
      const result = await keyImport({
        vault_id: vaultId,
        private_key: privateKey,
        ...(label.trim() === '' ? {} : { label: label.trim() }),
        ...(needPassphrase && passphrase !== '' ? { passphrase, save_passphrase: savePassphrase } : {}),
      });
      link(result.key);
    } catch (reason) {
      const error = toIpcError(reason);
      setPassphrase('');
      if (error.code !== 'validation_failed') {
        setErrors({ general: t(`errors.${error.code}`) });
        return;
      }
      const next: Errors = {};
      for (const field of error.fieldErrors) {
        if (field.field === 'private_key') {
          next.privateKey = privateKeyMessage(field.reason);
        } else if (field.field === 'passphrase') {
          if (field.reason === 'required') {
            setNeedPassphrase(true);
          } else if (field.reason === 'invalid') {
            setNeedPassphrase(true);
            next.passphrase = t('key_add.error.wrong_passphrase');
          } else {
            setNeedPassphrase(false);
            next.general = t('key_add.error.not_encrypted');
          }
        } else if (field.field === 'label') {
          next.label = t(`validation.${field.reason}`);
        } else {
          next.general = t('errors.validation_failed');
        }
      }
      if (error.fieldErrors.length === 0) {
        next.general = t('errors.validation_failed');
      }
      setErrors(next);
      if (next.passphrase !== undefined) {
        passphraseRef.current?.focus();
      }
    } finally {
      setSubmitting(false);
    }
  };

  const refuseCopy = (event: ClipboardEvent) => event.preventDefault();

  /** Enter in a single-line field adds the key. */
  const submitOnEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void submit();
    }
  };

  return (
    <Modal
      open
      size="medium"
      title={t('key_add.title')}
      closeLabel={t('common.close')}
      onClose={onClose}
      initialFocusRef={pasteRef}
      data-testid="key-add-dialog"
      footer={
        <>
          <Button onClick={onClose} data-testid="key-add-cancel">
            {t('common.cancel')}
          </Button>
          <Button variant="primary" loading={submitting} onClick={() => void submit()} data-testid="key-add-submit">
            {t('key_add.submit')}
          </Button>
        </>
      }
    >
      <form className={styles.keyDialog} onSubmit={(event) => void submit(event)} noValidate>
        {keys !== null && keys.length > 0 ? (
          <section className={styles.keySection} aria-labelledby={`${pasteId}-existing`}>
            <h3 id={`${pasteId}-existing`} className={styles.keySectionTitle}>
              {t('key_add.existing')}
            </h3>
            <ul className={styles.keyList} data-testid="key-add-existing">
              {keys.map((key) => (
                <li key={key.record_id}>
                  <button
                    type="button"
                    className={styles.keyItem}
                    title={key.fingerprint}
                    onClick={() => link(key)}
                    data-record-id={key.record_id}
                    data-testid="key-add-existing-item"
                  >
                    <KeyRound size={16} strokeWidth={1.75} aria-hidden="true" />
                    <span className={styles.keyItemLabel}>{key.label}</span>
                    <span className={styles.keyItemType}>{key.algorithm}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        <section className={styles.keySection} aria-label={t('key_add.new')}>
          {/* The heading separates the two ways only when saved keys are listed. */}
          {keys !== null && keys.length > 0 ? <h3 className={styles.keySectionTitle}>{t('key_add.new')}</h3> : null}
          <label htmlFor={pasteId} className={styles.keySectionTitle}>
            {t('key_add.paste_label')}
          </label>
          <textarea
            ref={pasteRef}
            id={pasteId}
            className={styles.keyText}
            value={privateKey}
            placeholder={t('key_add.paste_placeholder')}
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            aria-invalid={errors.privateKey !== undefined ? 'true' : undefined}
            aria-describedby={errors.privateKey !== undefined ? `${pasteId}-error` : undefined}
            onChange={(event) => changePrivateKey(event.target.value)}
            onCopy={refuseCopy}
            onCut={refuseCopy}
            data-sensitive=""
            data-testid="key-add-paste"
          />
          {errors.privateKey !== undefined ? (
            <p id={`${pasteId}-error`} className={styles.fieldError} data-testid="key-add-paste-error">
              {errors.privateKey}
            </p>
          ) : null}
          <div className={styles.fileRow}>
            {/* The native input is visually hidden (its text follows the system language); the button opens it. */}
            <input
              ref={fileRef}
              id={fileId}
              type="file"
              className={styles.fileInput}
              tabIndex={-1}
              aria-label={t('key_add.file_button')}
              onChange={(event) => void chooseFile(event)}
              data-testid="key-add-file"
            />
            <Button icon={<FileUp />} onClick={() => fileRef.current?.click()} data-testid="key-add-file-button">
              {t('key_add.file_button')}
            </Button>
            {fileName === null ? null : (
              <span className={styles.hint} data-testid="key-add-file-name">
                {fileName}
              </span>
            )}
          </div>
          <TextInput
            label={t('key_add.label')}
            placeholder={t('key_add.label_placeholder')}
            value={label}
            autoComplete="off"
            error={errors.label}
            onChange={(event) => {
              setLabel(event.target.value);
              setErrors((current) => ({ ...current, label: undefined }));
            }}
            onKeyDown={submitOnEnter}
            data-testid="key-add-label"
          />
        </section>

        {needPassphrase ? (
          <section className={styles.keySection}>
            <p className={styles.hint}>{t('key_add.passphrase_hint')}</p>
            <PasswordInput
              ref={passphraseRef}
              label={t('key_add.passphrase')}
              toggleLabel={t('prompt.show_value')}
              value={passphrase}
              visible={passphraseVisible}
              onVisibleChange={setPassphraseVisible}
              error={errors.passphrase}
              onChange={(event) => {
                setPassphrase(event.target.value);
                setErrors((current) => ({ ...current, passphrase: undefined }));
              }}
              onKeyDown={submitOnEnter}
              data-testid="key-add-passphrase"
            />
            <Checkbox
              checked={savePassphrase}
              onCheckedChange={setSavePassphrase}
              label={t('key_add.save_passphrase')}
              data-testid="key-add-save-passphrase"
            />
            <p className={styles.hint}>{savePassphrase ? t('key_add.save_passphrase_on') : t('key_add.save_passphrase_off')}</p>
          </section>
        ) : null}

        {errors.general !== undefined ? (
          <p className={styles.generalError} role="alert" data-testid="key-add-error">
            {errors.general}
          </p>
        ) : null}
      </form>
    </Modal>
  );
}
