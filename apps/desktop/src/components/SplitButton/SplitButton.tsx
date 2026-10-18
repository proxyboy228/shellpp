import { ChevronDown } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import type { FocusEvent, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { Button } from '../Button/Button';
import styles from './SplitButton.module.css';

export interface SplitButtonItem {
  /** Stable key; also used for the default item test id. */
  id: string;
  /** Visible text; also used for type-ahead. */
  label: string;
  /** Leading icon, for example a lucide-react icon element. */
  icon?: ReactNode;
  /** Shown but cannot be chosen; skipped by arrow-key navigation. */
  disabled?: boolean;
  /** A destructive item, drawn with the danger color. */
  danger?: boolean;
  /** Called after the menu has closed and focus has returned to the arrow. */
  onSelect: () => void;
  /** Test id of the item. Default: `<data-testid>-item-<id>` when the split button has a test id. */
  'data-testid'?: string;
}

export type SplitButtonVariant = 'primary' | 'secondary';

export interface SplitButtonProps {
  /** Content of the main part ("New host"). */
  children: ReactNode;
  /** The main action. */
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
  /** Items of the menu that the arrow opens. */
  items: readonly SplitButtonItem[];
  /** Accessible name of the arrow part. Default: "More actions". */
  menuLabel?: string;
  /** Default: `primary`. */
  variant?: SplitButtonVariant;
  /** Leading icon of the main part. */
  icon?: ReactNode;
  /** Disables both parts. */
  disabled?: boolean;
  /** Which edge of the split button the menu lines up with. Default: `start`. */
  menuAlign?: 'start' | 'end';
  className?: string;
  /**
   * Test id of the wrapper. The parts get `<id>-main`, `<id>-toggle` and
   * `<id>-menu`; items get `<id>-item-<item id>` unless they set their own.
   */
  'data-testid'?: string;
}

type FocusTarget = 'first' | 'last';

function findEnabled(items: readonly SplitButtonItem[], from: number, step: 1 | -1): number {
  const count = items.length;
  for (let offset = 1; offset <= count; offset += 1) {
    const index = (((from + step * offset) % count) + count) % count;
    if (!items[index].disabled) {
      return index;
    }
  }
  return -1;
}

function edgeEnabled(items: readonly SplitButtonItem[], target: FocusTarget): number {
  return target === 'first' ? findEnabled(items, -1, 1) : findEnabled(items, 0, -1);
}

/**
 * A main action plus an arrow that opens a short list of related actions
 * (menu button pattern: `aria-haspopup="menu"`, `role="menu"`, roving focus).
 */
export function SplitButton({
  children,
  onClick,
  items,
  menuLabel = 'More actions',
  variant = 'primary',
  icon,
  disabled = false,
  menuAlign = 'start',
  className,
  'data-testid': testId,
}: SplitButtonProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const rootRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const baseId = useId();
  const toggleId = `${baseId}-toggle`;
  const menuId = `${baseId}-menu`;
  const isOpen = open && !disabled;

  // Move focus to the active item whenever the menu opens or the active item changes.
  useEffect(() => {
    if (!isOpen) {
      return;
    }
    const target = activeIndex >= 0 ? itemRefs.current[activeIndex] : menuRef.current;
    target?.focus();
  }, [isOpen, activeIndex]);

  // Close on a pointer press outside the split button.
  useEffect(() => {
    if (!isOpen) {
      return undefined;
    }
    function handlePointerDown(event: PointerEvent) {
      const root = rootRef.current;
      if (root !== null && event.target instanceof Node && !root.contains(event.target)) {
        setOpen(false);
        setActiveIndex(-1);
      }
    }
    document.addEventListener('pointerdown', handlePointerDown);
    return () => document.removeEventListener('pointerdown', handlePointerDown);
  }, [isOpen]);

  function openMenu(target: FocusTarget) {
    setActiveIndex(edgeEnabled(items, target));
    setOpen(true);
  }

  function closeMenu(returnFocus: boolean) {
    setOpen(false);
    setActiveIndex(-1);
    if (returnFocus) {
      toggleRef.current?.focus();
    }
  }

  function handleToggleClick() {
    if (isOpen) {
      closeMenu(false);
    } else {
      openMenu('first');
    }
  }

  function handleToggleKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      openMenu(event.key === 'ArrowDown' ? 'first' : 'last');
    }
  }

  function handleMenuKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowUp': {
        event.preventDefault();
        const next = findEnabled(items, activeIndex, event.key === 'ArrowDown' ? 1 : -1);
        if (next >= 0) {
          setActiveIndex(next);
        }
        return;
      }
      case 'Home':
      case 'End':
        event.preventDefault();
        setActiveIndex(edgeEnabled(items, event.key === 'Home' ? 'first' : 'last'));
        return;
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        closeMenu(true);
        return;
      case 'Tab':
        // Close and continue the tab sequence from the arrow: Tab moves on to
        // the next control after the split button, Shift+Tab lands on the arrow.
        if (event.shiftKey) {
          event.preventDefault();
        }
        closeMenu(true);
        return;
      default:
        break;
    }
    // Type-ahead: a printable character moves to the next item starting with it.
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && event.key !== ' ') {
      const query = event.key.toLocaleLowerCase();
      const count = items.length;
      for (let offset = 1; offset <= count; offset += 1) {
        const index = (activeIndex + offset + count) % count;
        const item = items[index];
        if (!item.disabled && item.label.toLocaleLowerCase().startsWith(query)) {
          setActiveIndex(index);
          break;
        }
      }
    }
  }

  function handleBlur(event: FocusEvent<HTMLDivElement>) {
    if (!isOpen) {
      return;
    }
    const next = event.relatedTarget;
    const root = rootRef.current;
    if (next instanceof Node && root !== null && root.contains(next)) {
      return;
    }
    if (next !== null) {
      closeMenu(false);
    }
  }

  function handleItemClick(item: SplitButtonItem) {
    if (item.disabled) {
      return;
    }
    closeMenu(true);
    item.onSelect();
  }

  const rootClasses = [styles.root, className].filter(Boolean).join(' ');
  const menuClasses = [styles.menu, menuAlign === 'end' ? styles.alignEnd : undefined].filter(Boolean).join(' ');

  return (
    <div ref={rootRef} className={rootClasses} data-variant={variant} data-testid={testId} onBlur={handleBlur}>
      <Button
        variant={variant}
        icon={icon}
        disabled={disabled}
        className={styles.main}
        onClick={onClick}
        data-testid={testId === undefined ? undefined : `${testId}-main`}
      >
        {children}
      </Button>
      <Button
        ref={toggleRef}
        id={toggleId}
        variant={variant}
        icon={<ChevronDown className={styles.chevron} />}
        disabled={disabled}
        className={styles.toggle}
        aria-label={menuLabel}
        aria-haspopup="menu"
        aria-expanded={isOpen}
        aria-controls={isOpen ? menuId : undefined}
        onClick={handleToggleClick}
        onKeyDown={handleToggleKeyDown}
        data-testid={testId === undefined ? undefined : `${testId}-toggle`}
      />
      {isOpen ? (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-labelledby={toggleId}
          aria-orientation="vertical"
          tabIndex={-1}
          className={menuClasses}
          onKeyDown={handleMenuKeyDown}
          data-testid={testId === undefined ? undefined : `${testId}-menu`}
        >
          {items.map((item, index) => (
            <button
              key={item.id}
              ref={(element) => {
                itemRefs.current[index] = element;
              }}
              type="button"
              role="menuitem"
              tabIndex={-1}
              aria-disabled={item.disabled || undefined}
              className={[styles.item, item.danger ? styles.danger : undefined].filter(Boolean).join(' ')}
              onClick={() => handleItemClick(item)}
              data-testid={item['data-testid'] ?? (testId === undefined ? undefined : `${testId}-item-${item.id}`)}
            >
              {item.icon !== undefined ? (
                <span className={styles.itemIcon} aria-hidden="true">
                  {item.icon}
                </span>
              ) : null}
              <span className={styles.itemLabel}>{item.label}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export default SplitButton;
