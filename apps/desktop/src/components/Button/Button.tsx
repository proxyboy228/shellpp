import { LoaderCircle } from 'lucide-react';
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from 'react';
import styles from './Button.module.css';

export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'text';

export interface ButtonProps extends Omit<ComponentPropsWithRef<'button'>, 'type'> {
  /**
   * Visual role. `primary` is the single main action of a screen (accent),
   * `danger` a destructive action, `text` a link-style action. Default:
   * `secondary`.
   */
  variant?: ButtonVariant;
  /** Native button type. Default: `button`, so the button never submits a form by accident. */
  type?: 'button' | 'submit' | 'reset';
  /**
   * Shows a spinner in place of the icon and ignores activation. The button
   * stays focusable (`aria-disabled`) so keyboard focus is not lost while
   * the action runs.
   */
  loading?: boolean;
  /** Leading icon, for example a lucide-react icon element. Hidden from assistive technology. */
  icon?: ReactNode;
  /** Stretches the button to the width of its container. */
  fullWidth?: boolean;
  'data-testid'?: string;
}

/** Text button with the primary, secondary, danger and text variants (design-system §6). */
export function Button({
  variant = 'secondary',
  type = 'button',
  loading = false,
  icon,
  fullWidth = false,
  disabled = false,
  className,
  children,
  onClick,
  ...rest
}: ButtonProps) {
  const classes = [styles.button, styles[variant], fullWidth ? styles.fullWidth : undefined, className]
    .filter(Boolean)
    .join(' ');

  function handleClick(event: MouseEvent<HTMLButtonElement>) {
    if (loading) {
      event.preventDefault();
      return;
    }
    onClick?.(event);
  }

  const leading = loading ? <LoaderCircle className={styles.spinner} /> : icon;

  return (
    <button
      {...rest}
      type={type}
      className={classes}
      disabled={disabled}
      aria-disabled={loading || undefined}
      aria-busy={loading || undefined}
      data-variant={variant}
      onClick={handleClick}
    >
      {leading !== undefined && leading !== null && leading !== false ? (
        <span className={styles.icon} aria-hidden="true">
          {leading}
        </span>
      ) : null}
      {children !== undefined && children !== null && children !== false ? (
        <span className={styles.label}>{children}</span>
      ) : null}
    </button>
  );
}

export default Button;
