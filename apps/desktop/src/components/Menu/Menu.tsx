import { ChevronRight, Info, type LucideIcon } from 'lucide-react';
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type MouseEvent,
  type RefObject,
} from 'react';
import styles from './Menu.module.css';

export interface MenuItemDef {
  type?: 'item';
  /** Stable id, passed to `onAction`. */
  id: string;
  label: string;
  /** Leading icon from lucide-react. */
  icon?: LucideIcon;
  /** Shortcut hint shown at the right edge, for example `Ctrl+D`. */
  shortcut?: string;
  /** Dangerous action (Remove, Close): `--danger` text. */
  danger?: boolean;
  /** Inactive item: `--text-disabled` text, skipped by the keyboard, not activatable. */
  disabled?: boolean;
  /**
   * The item opens a submenu: shows the indicator, sets `aria-haspopup`,
   * and activating it (Enter, Space, Right, click) does not close the menu.
   */
  submenu?: boolean;
  /**
   * Explanation shown by an info icon at the end of the item: on hover of the
   * icon and while the item has keyboard focus. The icon gets
   * `<item test id>-info`; the text is also the item's accessible description.
   */
  info?: string;
  /** Called when the item is activated, before `onAction`. */
  onSelect?: () => void;
  'data-testid'?: string;
}

export interface MenuSeparatorDef {
  type: 'separator';
  id: string;
}

export type MenuEntry = MenuItemDef | MenuSeparatorDef;

export interface MenuProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'role' | 'onSelect'> {
  items: readonly MenuEntry[];
  /** Called with the id of an activated item. */
  onAction?: (id: string) => void;
  /**
   * Called on Escape, on Tab, after an item without a submenu is activated,
   * and on a pointer press outside the menu and `anchorRef`.
   */
  onClose?: () => void;
  /** Focus the first enabled item on mount. Default: true. */
  autoFocus?: boolean;
  /**
   * The trigger element (or several elements, for example the trigger and a
   * nested submenu): a press on them does not count as an outside press.
   */
  anchorRef?: RefObject<HTMLElement | null> | readonly RefObject<HTMLElement | null>[];
  'data-testid'?: string;
}

const ITEM_SELECTOR = '[role="menuitem"]:not([aria-disabled="true"])';

function getEnabledItems(menu: HTMLElement): HTMLElement[] {
  return Array.from(menu.querySelectorAll<HTMLElement>(ITEM_SELECTOR)).filter(
    (element) => element.closest('[role="menu"]') === menu,
  );
}

function isSeparator(entry: MenuEntry): entry is MenuSeparatorDef {
  return entry.type === 'separator';
}

function anchorList(
  anchorRef: MenuProps['anchorRef'],
): readonly RefObject<HTMLElement | null>[] {
  if (anchorRef === undefined) {
    return [];
  }
  return 'current' in anchorRef ? [anchorRef] : anchorRef;
}

/**
 * Popover menu (`role="menu"`): items, separators, danger, disabled and
 * submenu items. Keyboard: Up and Down move between enabled items (wrapping),
 * Home and End jump, Enter or Space activates, Right activates a submenu
 * item, Escape and Tab close, a letter moves to the next item starting with it.
 * Positioning is up to the parent.
 */
export function Menu({
  items,
  onAction,
  onClose,
  autoFocus = true,
  anchorRef,
  className,
  onKeyDown,
  'data-testid': testId,
  ...rest
}: MenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const infoIdPrefix = useId();
  // Info tooltips: the item whose info icon is hovered, and the focused item
  // while the keyboard is used (a hover also focuses an item).
  const [infoHover, setInfoHover] = useState<number | null>(null);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);
  const [keyboardMode, setKeyboardMode] = useState(false);
  const hasIcons = items.some((entry) => !isSeparator(entry) && entry.icon !== undefined);

  // Move focus into the menu on mount; give it back on unmount when it is
  // still inside (closed with the keyboard or by an item), so that a press
  // on another control keeps its focus. A layout effect runs its cleanup
  // before the menu leaves the DOM, while focus can still be checked.
  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (menu === null) {
      return undefined;
    }
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (autoFocus) {
      (getEnabledItems(menu)[0] ?? menu).focus();
    }
    return () => {
      const active = document.activeElement;
      const focusInside = active === null || active === document.body || menu.contains(active);
      if (focusInside && previous !== null && previous !== document.body && previous.isConnected) {
        previous.focus();
      }
    };
  }, [autoFocus]);

  useEffect(() => {
    if (onClose === undefined) {
      return undefined;
    }
    const handlePointerDown = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      if (menuRef.current?.contains(target) || anchorList(anchorRef).some((ref) => ref.current?.contains(target))) {
        return;
      }
      onClose();
    };
    document.addEventListener('pointerdown', handlePointerDown, true);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
    };
  }, [onClose, anchorRef]);

  const activate = (item: MenuItemDef) => {
    if (item.disabled) {
      return;
    }
    item.onSelect?.();
    onAction?.(item.id);
    if (!item.submenu) {
      onClose?.();
    }
  };

  const itemFromElement = (element: Element | null): MenuItemDef | undefined => {
    const attribute = element?.getAttribute('data-index') ?? null;
    const index = attribute === null ? Number.NaN : Number(attribute);
    const entry = Number.isInteger(index) ? items[index] : undefined;
    return entry === undefined || isSeparator(entry) ? undefined : entry;
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    onKeyDown?.(event);
    setKeyboardMode(true);
    const menu = menuRef.current;
    if (event.defaultPrevented || menu === null) {
      return;
    }
    const enabled = getEnabledItems(menu);
    const active = document.activeElement;
    const current = enabled.findIndex((element) => element === active);
    const focusAt = (index: number) => {
      enabled[index]?.focus();
    };

    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        if (enabled.length > 0) {
          focusAt(current < 0 ? 0 : (current + 1) % enabled.length);
        }
        break;
      case 'ArrowUp':
        event.preventDefault();
        if (enabled.length > 0) {
          focusAt(current < 0 ? enabled.length - 1 : (current - 1 + enabled.length) % enabled.length);
        }
        break;
      case 'Home':
        event.preventDefault();
        focusAt(0);
        break;
      case 'End':
        event.preventDefault();
        focusAt(enabled.length - 1);
        break;
      case 'Enter':
      case ' ': {
        const item = current < 0 ? undefined : itemFromElement(enabled[current] ?? null);
        if (item !== undefined) {
          event.preventDefault();
          activate(item);
        }
        break;
      }
      case 'ArrowRight': {
        const item = current < 0 ? undefined : itemFromElement(enabled[current] ?? null);
        if (item?.submenu) {
          event.preventDefault();
          activate(item);
        }
        break;
      }
      case 'Escape':
        event.preventDefault();
        // Do not let an enclosing modal window close as well.
        event.stopPropagation();
        onClose?.();
        break;
      case 'Tab':
        event.preventDefault();
        onClose?.();
        break;
      default:
        if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
          const key = event.key.toLocaleLowerCase();
          for (let step = 1; step <= enabled.length; step += 1) {
            const index = (current + step) % enabled.length;
            const label = itemFromElement(enabled[index] ?? null)?.label ?? '';
            if (label.toLocaleLowerCase().startsWith(key)) {
              focusAt(index);
              break;
            }
          }
        }
    }
  };

  const rootClassName = [styles.menu, className].filter(Boolean).join(' ');

  return (
    <div
      {...rest}
      ref={menuRef}
      role="menu"
      tabIndex={-1}
      className={rootClassName}
      data-testid={testId}
      onKeyDown={handleKeyDown}
      onMouseMove={() => setKeyboardMode(false)}
    >
      {items.map((entry, index) => {
        if (isSeparator(entry)) {
          return <div key={entry.id} role="separator" className={styles.separator} />;
        }
        const Icon = entry.icon;
        const infoId = entry.info === undefined ? undefined : `${infoIdPrefix}-info-${index}`;
        const infoVisible = infoHover === index || (keyboardMode && focusedIndex === index);
        const itemTestId = entry['data-testid'];
        const itemClassName = [
          styles.item,
          entry.danger ? styles.danger : undefined,
          entry.disabled ? styles.itemDisabled : undefined,
        ]
          .filter(Boolean)
          .join(' ');
        return (
          <div
            key={entry.id}
            role="menuitem"
            tabIndex={-1}
            className={itemClassName}
            aria-disabled={entry.disabled ? 'true' : undefined}
            aria-haspopup={entry.submenu ? 'menu' : undefined}
            aria-keyshortcuts={entry.shortcut}
            aria-describedby={infoId}
            data-index={index}
            data-danger={entry.danger ? 'true' : undefined}
            data-testid={entry['data-testid']}
            onMouseDown={(event: MouseEvent<HTMLDivElement>) => {
              // A press on an inactive item must not move focus to it.
              if (entry.disabled) {
                event.preventDefault();
              }
            }}
            onMouseEnter={(event: MouseEvent<HTMLDivElement>) => {
              if (!entry.disabled) {
                event.currentTarget.focus();
              }
            }}
            onFocus={() => setFocusedIndex(index)}
            onBlur={() => setFocusedIndex((current) => (current === index ? null : current))}
            onClick={() => activate(entry)}
          >
            {hasIcons ? (
              <span className={styles.icon} aria-hidden="true">
                {Icon === undefined ? null : <Icon size={16} strokeWidth={1.75} aria-hidden="true" focusable="false" />}
              </span>
            ) : null}
            <span className={styles.label}>{entry.label}</span>
            {entry.info === undefined ? null : (
              <span className={styles.infoWrap} aria-hidden="true">
                <span
                  className={styles.info}
                  data-testid={itemTestId === undefined ? undefined : `${itemTestId}-info`}
                  onMouseEnter={() => setInfoHover(index)}
                  onMouseLeave={() => setInfoHover((current) => (current === index ? null : current))}
                  onClick={(event) => {
                    // The icon explains the item; it does not activate it.
                    event.stopPropagation();
                    event.preventDefault();
                  }}
                >
                  <Info size={14} strokeWidth={1.75} aria-hidden="true" focusable="false" />
                </span>
                <span
                  id={infoId}
                  role="tooltip"
                  className={styles.tooltip}
                  hidden={!infoVisible}
                  data-testid={itemTestId === undefined ? undefined : `${itemTestId}-info-text`}
                >
                  {entry.info}
                </span>
              </span>
            )}
            {entry.shortcut === undefined ? null : (
              <span className={styles.shortcut} aria-hidden="true">
                {entry.shortcut}
              </span>
            )}
            {entry.submenu ? (
              <ChevronRight
                className={styles.submenuIndicator}
                size={16}
                strokeWidth={1.75}
                aria-hidden="true"
                focusable="false"
                data-testid={entry['data-testid'] === undefined ? undefined : `${entry['data-testid']}-submenu`}
              />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export default Menu;
