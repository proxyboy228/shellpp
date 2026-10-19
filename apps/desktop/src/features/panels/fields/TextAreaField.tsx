// Multi-line text field (host Notes, UI 3.1) styled like TextInput.

import { useId, type ChangeEvent } from 'react';
import styles from '../panels.module.css';

export interface TextAreaFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  /** Message under the field; marks it invalid. */
  error?: string;
  rows?: number;
  /** On the textarea; the error message gets `<id>-error`. */
  'data-testid': string;
}

export function TextAreaField({ label, value, onChange, error, rows = 3, 'data-testid': testId }: TextAreaFieldProps) {
  const id = useId();
  const errorId = `${id}-error`;
  return (
    <div className={styles.field} data-invalid={error === undefined ? undefined : true}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <textarea
        id={id}
        className={styles.textarea}
        value={value}
        rows={rows}
        aria-invalid={error === undefined ? undefined : true}
        aria-describedby={error === undefined ? undefined : errorId}
        onChange={(event: ChangeEvent<HTMLTextAreaElement>) => onChange(event.target.value)}
        data-testid={testId}
      />
      {error === undefined ? null : (
        <p id={errorId} className={styles.error} data-testid={`${testId}-error`}>
          {error}
        </p>
      )}
    </div>
  );
}

export default TextAreaField;
