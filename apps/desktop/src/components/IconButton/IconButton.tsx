import type { ComponentPropsWithRef, ReactNode } from 'react';
import styles from './IconButton.module.css';

export type IconButtonVariant = 'ghost' | 'secondary' | 'danger';
export type IconButtonSize = 'md' | 'sm';

export interface IconButtonProps
  extends Omit<ComponentPropsWithRef<'button'>, 'type' | 'children' | 'aria-label'> {
  /** Accessible name. Required: the button has no visible text. */
  'aria-label': string;
  /** The icon, for example a lucide-react icon element. Hidden from assistive technology. */
  icon: ReactNode;
  /**
   * `ghost` (default): no border, secondary icon color. `secondary`: with a
   * border, like the secondary Button. `danger`: a destructive action.
   */
  variant?: IconButtonVariant;
  /** `md` (default) is the control height; `sm` fits inside a field or a dense row. */
  size?: IconButtonSize;
  /**
   * Makes it a toggle button (`aria-pressed`), for example a view switch.
   * Leave undefined for a plain action button.
   */
  pressed?: boolean;
  /** Native button type. Default: `button`. */
  type?: 'button' | 'submit' | 'reset';
  'data-testid'?: string;
}

/** Square button that shows only an icon; the accessible name comes from `aria-label`. */
export function IconButton({
  icon,
  variant = 'ghost',
  size = 'md',
  pressed,
  type = 'button',
  className,
  ...rest
}: IconButtonProps) {
  const classes = [styles.iconButton, styles[variant], styles[size], className].filter(Boolean).join(' ');

  return (
    <button {...rest} type={type} className={classes} aria-pressed={pressed} data-variant={variant}>
      <span className={styles.icon} aria-hidden="true">
        {icon}
      </span>
    </button>
  );
}

export default IconButton;
