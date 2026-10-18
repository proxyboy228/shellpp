import { useId, useState } from 'react';
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from 'react';
import styles from './Toggle.module.css';

export interface ToggleProps
  extends Omit<
    ComponentPropsWithRef<'button'>,
    'type' | 'role' | 'children' | 'value' | 'onChange' | 'aria-checked' | 'defaultChecked'
  > {
  /** Controlled state. Omit to let the component keep its own state. */
  checked?: boolean;
  /** Initial state when uncontrolled. Default: `false`. */
  defaultChecked?: boolean;
  /** Called with the new state when the user flips the switch. */
  onCheckedChange?: (checked: boolean) => void;
  /**
   * Visible label, linked to the switch; clicking it flips the switch.
   * Without it, pass `aria-label` or `aria-labelledby`.
   */
  label?: ReactNode;
  /** Helper text under the label, linked with `aria-describedby`. */
  description?: ReactNode;
  /** Side of the label. Default: `end` (after the switch). */
  labelPosition?: 'start' | 'end';
  /** Class of the wrapper. */
  className?: string;
  /** Test id of the switch element. */
  'data-testid'?: string;
}

function hasContent(node: ReactNode): boolean {
  return node !== undefined && node !== null && node !== false && node !== '';
}

/** On/off switch (`role="switch"`). Space and Enter flip it, like a click. */
export function Toggle({
  checked,
  defaultChecked = false,
  onCheckedChange,
  label,
  description,
  labelPosition = 'end',
  className,
  id,
  disabled = false,
  onClick,
  'aria-describedby': ariaDescribedBy,
  ...rest
}: ToggleProps) {
  const [internalChecked, setInternalChecked] = useState(defaultChecked);
  const isChecked = checked ?? internalChecked;
  const generatedId = useId();
  const switchId = id ?? `${generatedId}-switch`;
  const descriptionId = `${switchId}-description`;
  const showLabel = hasContent(label);
  const showDescription = hasContent(description);

  const describedBy =
    [ariaDescribedBy, showDescription ? descriptionId : undefined].filter(Boolean).join(' ') || undefined;

  function handleClick(event: MouseEvent<HTMLButtonElement>) {
    onClick?.(event);
    if (event.defaultPrevented) {
      return;
    }
    const next = !isChecked;
    if (checked === undefined) {
      setInternalChecked(next);
    }
    onCheckedChange?.(next);
  }

  const rootClasses = [styles.root, labelPosition === 'start' ? styles.labelStart : undefined, className]
    .filter(Boolean)
    .join(' ');

  return (
    <div className={rootClasses} data-disabled={disabled || undefined}>
      <button
        {...rest}
        id={switchId}
        type="button"
        role="switch"
        aria-checked={isChecked}
        aria-describedby={describedBy}
        disabled={disabled}
        className={styles.switch}
        onClick={handleClick}
      >
        <span className={styles.thumb} aria-hidden="true" />
      </button>
      {showLabel || showDescription ? (
        <div className={styles.text}>
          {showLabel ? (
            <label htmlFor={switchId} className={styles.label}>
              {label}
            </label>
          ) : null}
          {showDescription ? (
            <p id={descriptionId} className={styles.description}>
              {description}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export default Toggle;
