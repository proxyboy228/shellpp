import { useId, type ReactNode } from 'react';
import styles from './Checkbox.module.css';

export interface CheckboxProps {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: ReactNode;
  disabled?: boolean;
  className?: string;
  id?: string;
  /** On the `<input type="checkbox">` element. */
  'data-testid'?: string;
}

/**
 * Checkbox with a label, built on the native input ("I saved the recovery
 * key", UI 13.1).
 *
 * Owner: W4 (lead's skeleton: final props, minimal styling).
 */
export function Checkbox({ checked, onCheckedChange, label, disabled = false, className, id, 'data-testid': testId }: CheckboxProps) {
  const generatedId = useId();
  const inputId = id ?? `${generatedId}-checkbox`;
  return (
    <div className={[styles.root, className].filter(Boolean).join(' ')} data-disabled={disabled || undefined}>
      <input
        id={inputId}
        type="checkbox"
        className={styles.input}
        checked={checked}
        disabled={disabled}
        data-testid={testId}
        onChange={(event) => onCheckedChange(event.target.checked)}
      />
      <label htmlFor={inputId} className={styles.label}>
        {label}
      </label>
    </div>
  );
}

export default Checkbox;
