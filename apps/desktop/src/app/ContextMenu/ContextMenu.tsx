// A positioned popover menu with nested submenus, built on the base `Menu`.
// Used for the app menu (UI 1.4), card context menus (UI 2.4, 3.4) and the
// "..." menu of the details panel.
//
// Owner: W4. The props are the W4/W5 contract.
//
// Behavior:
// - The menu opens at `at` (viewport coordinates) and is moved back inside the
//   viewport when it would overflow.
// - An item with `children` opens a submenu beside it: on hover, click, Enter,
//   Space or Right. Left or Escape closes the submenu and returns focus to the
//   item. The submenu has `data-testid` = `<item test id>-menu` and opens on
//   the left side when there is no room on the right.
// - Activating a leaf item (at any level) calls its `onSelect`, then
//   `onAction(id)`, then `onClose()`.
// - Escape at the top level, Tab, and a press outside the menu (and outside
//   `anchorRef`) call `onClose()`. Focus returns to the element that had it
//   when the menu opened.

import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type RefObject,
} from 'react';
import { Menu, type MenuEntry, type MenuItemDef, type MenuSeparatorDef } from '../../components';
import styles from './ContextMenu.module.css';

export interface ContextMenuItem extends MenuItemDef {
  /**
   * Submenu entries. The item shows the submenu indicator; activating it
   * opens a nested menu beside it with `data-testid` = `<item test id>-menu`.
   */
  children?: readonly ContextMenuEntry[];
  // `info?: string` (from MenuItemDef): an info icon `<item test id>-info`
  // that shows the text on hover and keyboard focus.
}

export type ContextMenuEntry = ContextMenuItem | MenuSeparatorDef;

export interface ContextMenuProps {
  items: readonly ContextMenuEntry[];
  /** Viewport coordinates of the top-left corner (pointer position or trigger corner); clamped to the viewport. */
  at: { x: number; y: number };
  /** Escape, outside press, or after an item without children was activated. */
  onClose: () => void;
  /** Called with the id of any activated leaf item (after its own `onSelect`). */
  onAction?: (id: string) => void;
  /** The element that opened the menu (for example a menu button): a press on it is not an outside press. */
  anchorRef?: RefObject<HTMLElement | null>;
  /** Accessible name of the top-level menu. */
  'aria-label'?: string;
  'data-testid': string;
}

/** Distance kept from the viewport edges, in px (the 8 px spacing step). */
const VIEWPORT_MARGIN = 8;

function hasChildren(entry: ContextMenuEntry): entry is ContextMenuItem & { children: readonly ContextMenuEntry[] } {
  return entry.type !== 'separator' && entry.children !== undefined && entry.children.length > 0;
}

interface Placement {
  /** Preferred left edge. */
  x: number;
  y: number;
  /** For submenus: the right edge to use when there is no room on the right. */
  flipRight?: number;
}

/** Measures the element and moves it inside the viewport. */
function useClampedPosition(ref: RefObject<HTMLElement | null>, placement: Placement): { left: number; top: number } {
  const [position, setPosition] = useState({ left: placement.x, top: placement.y });
  const { x, y, flipRight } = placement;
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null) {
      return;
    }
    const rect = element.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    let left = x;
    let top = y;
    if (rect.width > 0 && left + rect.width > viewportWidth - VIEWPORT_MARGIN) {
      left =
        flipRight !== undefined && flipRight - rect.width >= VIEWPORT_MARGIN
          ? flipRight - rect.width
          : viewportWidth - VIEWPORT_MARGIN - rect.width;
    }
    if (rect.height > 0 && top + rect.height > viewportHeight - VIEWPORT_MARGIN) {
      top = viewportHeight - VIEWPORT_MARGIN - rect.height;
    }
    left = Math.max(VIEWPORT_MARGIN, left);
    top = Math.max(VIEWPORT_MARGIN, top);
    setPosition((current) => (current.left === left && current.top === top ? current : { left, top }));
  }, [ref, x, y, flipRight]);
  return position;
}

interface LevelProps {
  items: readonly ContextMenuEntry[];
  placement: Placement;
  testId: string | undefined;
  ariaLabel?: string;
  autoFocus: boolean;
  /** The whole context menu (every level): presses inside it are not outside presses. */
  anchors: readonly RefObject<HTMLElement | null>[];
  /** Close this level only (submenus) or everything (top level). */
  onCloseLevel: () => void;
  onCloseAll: () => void;
  onLeaf: (id: string) => void;
  isTopLevel: boolean;
}

interface OpenSubmenu {
  index: number;
  placement: Placement;
  byKeyboard: boolean;
  /** Changes to mount the submenu again (keyboard entry into a submenu opened by hover). */
  mount: number;
}

function MenuLevel({
  items,
  placement,
  testId,
  ariaLabel,
  autoFocus,
  anchors,
  onCloseLevel,
  onCloseAll,
  onLeaf,
  isTopLevel,
}: LevelProps) {
  const levelRef = useRef<HTMLDivElement>(null);
  const position = useClampedPosition(levelRef, placement);
  const [open, setOpen] = useState<OpenSubmenu | null>(null);
  const lastInputWasKeyboard = useRef(false);
  const leafActivated = useRef(false);
  const closeAllOnClose = useRef(false);

  const menuElement = useCallback((): HTMLElement | null => {
    const level = levelRef.current;
    if (level === null) {
      return null;
    }
    return level.querySelector<HTMLElement>(':scope > [role="menu"]');
  }, []);

  const itemElement = useCallback(
    (index: number): HTMLElement | null =>
      menuElement()?.querySelector<HTMLElement>(`:scope > [role="menuitem"][data-index="${index}"]`) ?? null,
    [menuElement],
  );

  const openAt = useCallback(
    (index: number, byKeyboard: boolean) => {
      const element = itemElement(index);
      const rect = element?.getBoundingClientRect();
      const nextPlacement: Placement =
        rect === undefined
          ? { x: position.left, y: position.top }
          : { x: rect.right - VIEWPORT_MARGIN / 2, y: rect.top - VIEWPORT_MARGIN / 2, flipRight: rect.left + VIEWPORT_MARGIN / 2 };
      setOpen((current) => {
        if (current !== null && current.index === index) {
          // Already open: Enter or Right moves focus into it.
          return byKeyboard ? { ...current, byKeyboard: true, mount: current.mount + 1 } : current;
        }
        return { index, placement: nextPlacement, byKeyboard, mount: 0 };
      });
    },
    [itemElement, position.left, position.top],
  );

  // Closing a submenu: the nested base menu gives focus back to the item that
  // had it when the submenu opened (the submenu item itself).
  const closeSubmenu = useCallback(() => {
    setOpen(null);
  }, []);

  // Items handed to the base menu: submenu items get the indicator and open
  // their submenu instead of closing the menu.
  const entries = useMemo<readonly MenuEntry[]>(
    () =>
      items.map((entry, index): MenuEntry => {
        if (entry.type === 'separator') {
          return entry;
        }
        const { children, ...item } = entry;
        void children;
        if (!hasChildren(entry)) {
          return { ...item, submenu: false };
        }
        return {
          ...item,
          submenu: true,
          onSelect: () => {
            entry.onSelect?.();
            openAt(index, lastInputWasKeyboard.current);
          },
        };
      }),
    [items, openAt],
  );

  const handleMouseOver = (event: MouseEvent<HTMLDivElement>) => {
    const target = event.target instanceof Element ? event.target : null;
    const item = target?.closest('[role="menuitem"]');
    if (item === null || item === undefined || item.closest('[role="menu"]') !== menuElement()) {
      return;
    }
    const index = Number(item.getAttribute('data-index'));
    const entry = items[index];
    if (entry !== undefined && hasChildren(entry) && !entry.disabled) {
      if (open?.index !== index) {
        openAt(index, false);
      }
    } else if (open !== null) {
      closeSubmenu();
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Tab') {
      closeAllOnClose.current = true;
    }
    if (!isTopLevel && event.key === 'ArrowLeft') {
      event.preventDefault();
      onCloseLevel();
    }
  };

  const handleMenuClose = () => {
    if (isTopLevel || leafActivated.current || closeAllOnClose.current) {
      leafActivated.current = false;
      closeAllOnClose.current = false;
      onCloseAll();
    } else {
      onCloseLevel();
    }
  };

  const handleAction = (id: string) => {
    const entry = items.find((candidate) => candidate.type !== 'separator' && candidate.id === id);
    if (entry !== undefined && !hasChildren(entry)) {
      leafActivated.current = true;
      onLeaf(id);
    }
  };

  const openEntry = open === null ? undefined : items[open.index];
  const openItem = openEntry !== undefined && hasChildren(openEntry) ? openEntry : undefined;

  return (
    <div
      ref={levelRef}
      className={styles.level}
      style={{ left: position.left, top: position.top }}
      onKeyDownCapture={() => {
        lastInputWasKeyboard.current = true;
      }}
      onMouseDownCapture={() => {
        lastInputWasKeyboard.current = false;
      }}
      onMouseOver={handleMouseOver}
    >
      <Menu
        items={entries}
        onClose={handleMenuClose}
        onAction={handleAction}
        onKeyDown={handleKeyDown}
        autoFocus={autoFocus}
        anchorRef={anchors}
        aria-label={ariaLabel}
        data-testid={testId}
      />
      {open !== null && openItem !== undefined ? (
        <MenuLevel
          key={`${open.index}-${open.mount}`}
          items={openItem.children}
          placement={open.placement}
          testId={openItem['data-testid'] === undefined ? undefined : `${openItem['data-testid']}-menu`}
          ariaLabel={openItem.label}
          autoFocus={open.byKeyboard}
          anchors={anchors}
          onCloseLevel={closeSubmenu}
          onCloseAll={onCloseAll}
          onLeaf={onLeaf}
          isTopLevel={false}
        />
      ) : null}
    </div>
  );
}

export function ContextMenu({
  items,
  at,
  onClose,
  onAction,
  anchorRef,
  'aria-label': ariaLabel,
  'data-testid': testId,
}: ContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  // Read during the first render, before the menu moves focus into itself.
  const [previousFocus] = useState<HTMLElement | null>(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  const anchors = useMemo<readonly RefObject<HTMLElement | null>[]>(
    () => (anchorRef === undefined ? [rootRef] : [rootRef, anchorRef]),
    [anchorRef],
  );

  // Give focus back when the menu closes while focus is inside it (this
  // cleanup runs before the cleanups of the nested menus).
  useLayoutEffect(() => {
    const root = rootRef.current;
    return () => {
      const active = document.activeElement;
      const inside = active === null || active === document.body || (root !== null && root.contains(active));
      if (inside && previousFocus !== null && previousFocus !== document.body && previousFocus.isConnected) {
        previousFocus.focus();
      }
    };
  }, [previousFocus]);

  const placement = useMemo<Placement>(() => ({ x: at.x, y: at.y }), [at.x, at.y]);

  return (
    <div ref={rootRef} className={styles.root} data-context-menu="">
      <MenuLevel
        items={items}
        placement={placement}
        testId={testId}
        ariaLabel={ariaLabel}
        autoFocus
        anchors={anchors}
        onCloseLevel={onClose}
        onCloseAll={onClose}
        onLeaf={(id) => onAction?.(id)}
        isTopLevel
      />
    </div>
  );
}

export default ContextMenu;
