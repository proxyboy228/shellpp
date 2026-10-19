import { useId, type ReactNode } from 'react';
import styles from './Select.module.css';

export interface SelectOption<T extends string = string> {
  value: T;
  label: string;
  disabled?: boolean;
}

export interface SelectProps<T extends string = string> {
  options: readonly SelectOption<T>[];
  value: T;
  onValueChange: (value: T) => void;
  label?: ReactNode;
  description?: ReactNode;
  /** Error message under the field (string) or only the invalid state (`true`). */
  error?: ReactNode;
  disabled?: boolean;
  className?: string;
  id?: string;
  /** On the `<select>` element; the error message gets `<id>-error`. */
  'data-testid'?: string;
}

function hasContent(node: ReactNode): boolean {
  return node !== undefined && node !== null && node !== false && node !== '';
}

/**
 * Dropdown built on the native `<select>` element (keyboard and screen reader
 * support from the platform; WebDriver selects options by value).
 *
 * Owner: W4 (lead's skeleton: final props, minimal styling).
 */
export function Select<T extends string = string>({
  options,
  value,
  onValueChange,
  label,
  description,
  error,
  disabled = false,
  className,
  id,
  'data-testid': testId,
}: SelectProps<T>) {
  const generatedId = useId();
  const selectId = id ?? `${generatedId}-select`;
  const descriptionId = `${selectId}-description`;
  const errorId = `${selectId}-error`;
  const invalid = hasContent(error);
  const showError = invalid && error !== true;
  const describedBy =
    [hasContent(description) ? descriptionId : undefined, showError ? errorId : undefined].filter(Boolean).join(' ') ||
    undefined;

  return (
    <div className={[styles.root, className].filter(Boolean).join(' ')} data-disabled={disabled || undefined}>
      {hasContent(label) ? (
        <label htmlFor={selectId} className={styles.label}>
          {label}
        </label>
      ) : null}
      <select
        id={selectId}
        className={styles.select}
        value={value}
        disabled={disabled}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
        data-testid={testId}
        onChange={(event) => onValueChange(event.target.value as T)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      {hasContent(description) ? (
        <p id={descriptionId} className={styles.description}>
          {description}
        </p>
      ) : null}
      {showError ? (
        <p id={errorId} className={styles.error} data-testid={testId === undefined ? undefined : `${testId}-error`}>
          {error}
        </p>
      ) : null}
    </div>
  );
}

export default Select;
