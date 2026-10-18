import { useId } from 'react';
import type { ComponentPropsWithRef, ReactNode } from 'react';
import styles from './TextInput.module.css';

export type TextInputType = 'text' | 'email' | 'url' | 'search' | 'tel' | 'number' | 'password';

export interface TextInputProps extends Omit<ComponentPropsWithRef<'input'>, 'type' | 'size' | 'children'> {
  /** Default: `text`. For secrets use PasswordInput, which adds the show/hide toggle. */
  type?: TextInputType;
  /**
   * Visible label, linked to the input. Without it, pass `aria-label` or
   * `aria-labelledby` so the field still has an accessible name.
   */
  label?: ReactNode;
  /** Helper text under the field, linked with `aria-describedby`. */
  description?: ReactNode;
  /**
   * Error message under the field. Any value other than `undefined`, `null`,
   * `false` or `''` marks the field invalid (`aria-invalid`); `true` marks it
   * invalid without a message.
   */
  error?: ReactNode;
  /** Monospace text, for keys, scripts and paths. */
  mono?: boolean;
  /** Decorative icon at the start of the field. Hidden from assistive technology. */
  leadingIcon?: ReactNode;
  /** Content at the end of the field, for example an IconButton with `size="sm"`. */
  trailing?: ReactNode;
  /** Class of the wrapper; `inputClassName` styles the input itself. */
  className?: string;
  inputClassName?: string;
  /** Test id of the input element; the error message gets `<id>-error`. */
  'data-testid'?: string;
}

function hasContent(node: ReactNode): boolean {
  return node !== undefined && node !== null && node !== false && node !== '';
}

/** Single-line text field with an optional label, helper text and error message. */
export function TextInput({
  type = 'text',
  label,
  description,
  error,
  mono = false,
  leadingIcon,
  trailing,
  className,
  inputClassName,
  id,
  disabled = false,
  'aria-describedby': ariaDescribedBy,
  'aria-invalid': ariaInvalid,
  'data-testid': testId,
  ...rest
}: TextInputProps) {
  const generatedId = useId();
  const inputId = id ?? `${generatedId}-input`;
  const descriptionId = `${inputId}-description`;
  const errorId = `${inputId}-error`;

  const invalid = hasContent(error);
  const showError = invalid && error !== true;
  const showDescription = hasContent(description);

  const describedBy =
    [ariaDescribedBy, showDescription ? descriptionId : undefined, showError ? errorId : undefined]
      .filter(Boolean)
      .join(' ') || undefined;

  const rootClasses = [styles.root, className].filter(Boolean).join(' ');
  const inputClasses = [styles.input, mono ? styles.mono : undefined, inputClassName].filter(Boolean).join(' ');

  return (
    <div className={rootClasses} data-disabled={disabled || undefined} data-invalid={invalid || undefined}>
      {hasContent(label) ? (
        <label htmlFor={inputId} className={styles.label}>
          {label}
        </label>
      ) : null}
      <div className={styles.box}>
        {hasContent(leadingIcon) ? (
          <span className={styles.leading} aria-hidden="true">
            {leadingIcon}
          </span>
        ) : null}
        <input
          {...rest}
          id={inputId}
          type={type}
          disabled={disabled}
          className={inputClasses}
          aria-invalid={invalid ? true : ariaInvalid}
          aria-describedby={describedBy}
          data-testid={testId}
        />
        {hasContent(trailing) ? <span className={styles.trailing}>{trailing}</span> : null}
      </div>
      {showDescription ? (
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

export default TextInput;
