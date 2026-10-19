// Environment variables of a host (UI 3.1 "Show more"): name and value rows.

import { Plus, X } from 'lucide-react';
import { Button, IconButton, TextInput } from '../../../components';
import { t } from '../../../i18n';
import { reasonFor } from '../form';
import type { EnvRow, PathError } from '../validators';
import styles from '../panels.module.css';

export interface EnvVarsFieldProps {
  rows: readonly EnvRow[];
  onChange: (rows: EnvRow[]) => void;
  /** Errors with paths `ssh.env.<i>.name`, `ssh.env.<i>.value`, `ssh.env`. */
  errors: readonly PathError[];
  /** Prefix: `<id>-env`, `<id>-env-add`, `<id>-env-row`, `<id>-env-name`, `<id>-env-value`, `<id>-env-remove`. */
  testIdPrefix: string;
}

export function EnvVarsField({ rows, onChange, errors, testIdPrefix }: EnvVarsFieldProps) {
  const update = (index: number, patch: Partial<EnvRow>) => {
    onChange(rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };
  const message = (path: string) => {
    const reason = reasonFor(errors, path);
    return reason === undefined ? undefined : t(`validation.${reason}`);
  };
  const listError = message('ssh.env');

  return (
    <div className={styles.field} data-testid={`${testIdPrefix}-env`}>
      <span className={styles.label}>{t('host_panel.env.label')}</span>
      {rows.map((row, index) => (
        <div key={index} className={styles.envRow} data-index={index} data-testid={`${testIdPrefix}-env-row`}>
          <TextInput
            aria-label={t('host_panel.env.name')}
            placeholder={t('host_panel.env.name')}
            value={row.name}
            mono
            spellCheck={false}
            autoComplete="off"
            error={message(`ssh.env.${index}.name`)}
            onChange={(event) => update(index, { name: event.target.value })}
            data-testid={`${testIdPrefix}-env-name`}
          />
          <TextInput
            aria-label={t('host_panel.env.value')}
            placeholder={t('host_panel.env.value')}
            value={row.value}
            mono
            spellCheck={false}
            autoComplete="off"
            error={message(`ssh.env.${index}.value`)}
            onChange={(event) => update(index, { value: event.target.value })}
            data-testid={`${testIdPrefix}-env-value`}
          />
          <IconButton
            icon={<X />}
            aria-label={t('host_panel.env.remove')}
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
            data-testid={`${testIdPrefix}-env-remove`}
          />
        </div>
      ))}
      {listError === undefined ? null : <p className={styles.error}>{listError}</p>}
      <div>
        <Button
          variant="text"
          icon={<Plus />}
          onClick={() => onChange([...rows, { name: '', value: '' }])}
          data-testid={`${testIdPrefix}-env-add`}
        >
          {t('host_panel.env.add')}
        </Button>
      </div>
    </div>
  );
}

export default EnvVarsField;
