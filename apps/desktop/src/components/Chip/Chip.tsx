import { X } from 'lucide-react';
import type { HTMLAttributes, MouseEvent } from 'react';
import styles from './Chip.module.css';

export interface ChipProps extends Omit<HTMLAttributes<HTMLSpanElement>, 'children' | 'onClick'> {
  /** Text of the chip, for example a tag name. */
  label: string;
  /**
   * Active filter chip: `--accent-subtle` fill with `--text-primary` text
   * (design-system 3.4). Tag chips are neutral otherwise.
   */
  active?: boolean;
  /** Inactive chip: opacity 0.45, no hover, its buttons do nothing. */
  disabled?: boolean;
  /**
   * Makes the label a toggle button (`aria-pressed` follows `active`),
   * for filter chips.
   */
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
  /** Shows the remove button and is called when it is activated. */
  onRemove?: () => void;
  /** Accessible name of the remove button. Default: `Remove <label>`. */
  removeLabel?: string;
  /** Test id of the root element; the remove button gets `<id>-remove`. */
  'data-testid'?: string;
}

/** Tag chip with an optional remove button (design-system 3.4). */
export function Chip({
  label,
  active = false,
  disabled = false,
  onClick,
  onRemove,
  removeLabel,
  className,
  'data-testid': testId,
  ...rest
}: ChipProps) {
  const rootClassName = [
    styles.chip,
    active ? styles.active : undefined,
    disabled ? styles.disabled : undefined,
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <span
      {...rest}
      className={rootClassName}
      data-active={active ? 'true' : undefined}
      aria-disabled={disabled ? 'true' : undefined}
      data-testid={testId}
    >
      {onClick === undefined ? (
        <span className={styles.label}>{label}</span>
      ) : (
        <button
          type="button"
          className={`${styles.label} ${styles.labelButton}`}
          aria-pressed={active}
          disabled={disabled}
          onClick={onClick}
        >
          {label}
        </button>
      )}
      {onRemove === undefined ? null : (
        <button
          type="button"
          className={styles.remove}
          aria-label={removeLabel ?? `Remove ${label}`}
          disabled={disabled}
          onClick={() => onRemove()}
          data-testid={testId === undefined ? undefined : `${testId}-remove`}
        >
          <X size={12} strokeWidth={2} aria-hidden="true" focusable="false" />
        </button>
      )}
    </span>
  );
}

export default Chip;
