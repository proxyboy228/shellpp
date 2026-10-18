import { useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import styles from './SegmentedControl.module.css';

export interface SegmentedControlOption<T extends string = string> {
  value: T;
  /** Visible text of the segment. */
  label?: ReactNode;
  /** Icon of the segment, for example a lucide-react icon element. Hidden from assistive technology. */
  icon?: ReactNode;
  /** Accessible name; required when the segment shows only an icon. */
  'aria-label'?: string;
  disabled?: boolean;
  /** Test id of the segment. Default: `<data-testid>-<value>` when the control has a test id. */
  'data-testid'?: string;
}

export interface SegmentedControlProps<T extends string = string> {
  options: readonly SegmentedControlOption<T>[];
  /** Controlled selection. Omit to let the component keep its own state. */
  value?: T;
  /** Initial selection when uncontrolled. */
  defaultValue?: T;
  /** Called with the newly selected value. */
  onValueChange?: (value: T) => void;
  /** Accessible name of the group; pass this or `aria-labelledby`. */
  'aria-label'?: string;
  'aria-labelledby'?: string;
  /** Disables every segment. */
  disabled?: boolean;
  /** Stretches the control and shares the width equally between segments. */
  fullWidth?: boolean;
  className?: string;
  /** Test id of the group; segments get `<id>-<value>` unless they set their own. */
  'data-testid'?: string;
}

/**
 * A row of mutually exclusive segments with radio group semantics
 * (`role="radiogroup"` and `role="radio"`). Arrow keys move the selection
 * and wrap around; Home and End jump to the ends; only the selected segment
 * is in the tab order.
 */
export function SegmentedControl<T extends string = string>({
  options,
  value,
  defaultValue,
  onValueChange,
  'aria-label': ariaLabel,
  'aria-labelledby': ariaLabelledBy,
  disabled = false,
  fullWidth = false,
  className,
  'data-testid': testId,
}: SegmentedControlProps<T>) {
  const [internalValue, setInternalValue] = useState<T | undefined>(defaultValue);
  const selected = value ?? internalValue;
  const segmentRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const isEnabled = (option: SegmentedControlOption<T>) => !disabled && !option.disabled;
  const selectedIndex = options.findIndex((option) => option.value === selected);
  // Roving tab stop: the selected segment, or the first enabled one when nothing is selected.
  const tabStopIndex =
    selectedIndex >= 0 && isEnabled(options[selectedIndex]) ? selectedIndex : options.findIndex(isEnabled);

  function select(index: number) {
    const option = options[index];
    if (option === undefined || !isEnabled(option)) {
      return;
    }
    if (option.value !== selected) {
      if (value === undefined) {
        setInternalValue(option.value);
      }
      onValueChange?.(option.value);
    }
  }

  function moveTo(index: number) {
    segmentRefs.current[index]?.focus();
    select(index);
  }

  function findEnabled(from: number, step: 1 | -1): number {
    const count = options.length;
    for (let offset = 1; offset <= count; offset += 1) {
      const index = (((from + step * offset) % count) + count) % count;
      if (isEnabled(options[index])) {
        return index;
      }
    }
    return -1;
  }

  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let target: number;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        target = findEnabled(index, 1);
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        target = findEnabled(index, -1);
        break;
      case 'Home':
        target = findEnabled(-1, 1);
        break;
      case 'End':
        target = findEnabled(0, -1);
        break;
      default:
        return;
    }
    event.preventDefault();
    if (target >= 0) {
      moveTo(target);
    }
  }

  const rootClasses = [styles.root, fullWidth ? styles.fullWidth : undefined, className].filter(Boolean).join(' ');

  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      aria-disabled={disabled || undefined}
      className={rootClasses}
      data-disabled={disabled || undefined}
      data-testid={testId}
    >
      {options.map((option, index) => {
        const isSelected = index === selectedIndex;
        return (
          <button
            key={option.value}
            ref={(element) => {
              segmentRefs.current[index] = element;
            }}
            type="button"
            role="radio"
            aria-checked={isSelected}
            aria-label={option['aria-label']}
            disabled={disabled || option.disabled}
            tabIndex={index === tabStopIndex ? 0 : -1}
            className={styles.segment}
            onClick={() => select(index)}
            onKeyDown={(event) => handleKeyDown(event, index)}
            data-testid={option['data-testid'] ?? (testId === undefined ? undefined : `${testId}-${option.value}`)}
          >
            {option.icon !== undefined && option.icon !== null ? (
              <span className={styles.icon} aria-hidden="true">
                {option.icon}
              </span>
            ) : null}
            {option.label !== undefined && option.label !== null ? (
              <span className={styles.label}>{option.label}</span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}

export default SegmentedControl;
