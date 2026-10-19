// Details panel (UI 1.3): a slide-out panel on the right of the Vaults tab
// that narrows the main area instead of overlapping it. Header: title, a "..."
// menu (optional) and a collapse button. Width: setting
// `view.details_panel_width` (320-560 px, default 380), changed by dragging the
// left edge (or with the arrow keys on the focused edge).
//
// Owner: W4. The props are the W4/W5 contract.

import { Ellipsis, PanelRightClose } from 'lucide-react';
import { useEffect, useId, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { IconButton } from '../../components';
import { t } from '../../i18n';
import { settingsSet } from '../../ipc';
import { ContextMenu, type ContextMenuEntry } from '../ContextMenu/ContextMenu';
import { numberSetting, settingsStore, useStoreSelector } from '../store';
import styles from './DetailsPanel.module.css';

export const DETAILS_PANEL_WIDTH_KEY = 'view.details_panel_width';
export const DETAILS_PANEL_MIN_WIDTH = 320;
export const DETAILS_PANEL_MAX_WIDTH = 560;
export const DETAILS_PANEL_DEFAULT_WIDTH = 380;
/** Width change per arrow key press on the resize handle. */
const KEYBOARD_STEP = 16;

export interface DetailsPanelProps {
  /** Header title, for example "Host details" or "New host". */
  title: string;
  /**
   * Entries of the "..." menu; the menu button is hidden when omitted or empty.
   * Items may have `children` (a submenu, for example "Move to") and `info`.
   */
  menuItems?: readonly ContextMenuEntry[];
  /** Collapse button and Escape. */
  onCollapse: () => void;
  children: ReactNode;
  /** Content pinned to the bottom (for example the Connect button, stage 2). */
  footer?: ReactNode;
  /**
   * Test id prefix. Default `details-panel`. Sub-elements: `<id>-title`,
   * `<id>-menu-button`, `<id>-menu`, `<id>-collapse`, `<id>-resize-handle`, `<id>-body`.
   */
  'data-testid'?: string;
}

export function clampPanelWidth(width: number): number {
  if (!Number.isFinite(width)) {
    return DETAILS_PANEL_DEFAULT_WIDTH;
  }
  return Math.min(DETAILS_PANEL_MAX_WIDTH, Math.max(DETAILS_PANEL_MIN_WIDTH, Math.round(width)));
}

const selectWidth = (settings: Parameters<typeof numberSetting>[0]) =>
  clampPanelWidth(numberSetting(settings, DETAILS_PANEL_WIDTH_KEY, DETAILS_PANEL_DEFAULT_WIDTH));

/** Writes the width setting; the stored value updates when the core answers. */
async function persistWidth(width: number): Promise<void> {
  const values = await settingsSet({ values: { [DETAILS_PANEL_WIDTH_KEY]: width } });
  settingsStore.set(values);
}

export function DetailsPanel({
  title,
  menuItems,
  onCollapse,
  children,
  footer,
  'data-testid': testId = 'details-panel',
}: DetailsPanelProps) {
  const titleId = useId();
  const storedWidth = useStoreSelector(settingsStore, selectWidth);
  // The width while dragging (or until the new setting arrives).
  const [pendingWidth, setPendingWidth] = useState<number | null>(null);
  const width = pendingWidth ?? storedWidth;
  const [dragging, setDragging] = useState(false);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const dragRef = useRef<{ startX: number; startWidth: number; latest: number } | null>(null);
  const cleanupDragRef = useRef<(() => void) | null>(null);
  const hasMenu = menuItems !== undefined && menuItems.length > 0;

  useEffect(
    () => () => {
      cleanupDragRef.current?.();
    },
    [],
  );

  const commitWidth = (next: number) => {
    const clamped = clampPanelWidth(next);
    setPendingWidth(clamped);
    if (clamped === storedWidth) {
      setPendingWidth(null);
      return;
    }
    Promise.resolve()
      .then(() => persistWidth(clamped))
      .catch(() => undefined)
      .finally(() => setPendingWidth(null));
  };

  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    dragRef.current = { startX: event.clientX, startWidth: width, latest: width };
    setPendingWidth(width);
    setDragging(true);
    const handleMove = (moveEvent: globalThis.PointerEvent) => {
      const drag = dragRef.current;
      if (drag === null) {
        return;
      }
      // The panel is on the right: moving the edge to the left widens it.
      drag.latest = clampPanelWidth(drag.startWidth + (drag.startX - moveEvent.clientX));
      setPendingWidth(drag.latest);
    };
    const finish = () => {
      const drag = dragRef.current;
      cleanup();
      if (drag !== null) {
        commitWidth(drag.latest);
      }
    };
    const cleanup = () => {
      dragRef.current = null;
      setDragging(false);
      window.removeEventListener('pointermove', handleMove);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      cleanupDragRef.current = null;
    };
    window.addEventListener('pointermove', handleMove);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
    cleanupDragRef.current = cleanup;
  };

  const handleResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const steps: Partial<Record<string, number>> = {
      ArrowLeft: width + KEYBOARD_STEP,
      ArrowRight: width - KEYBOARD_STEP,
      Home: DETAILS_PANEL_MIN_WIDTH,
      End: DETAILS_PANEL_MAX_WIDTH,
    };
    const next = steps[event.key];
    if (next !== undefined) {
      event.preventDefault();
      event.stopPropagation();
      commitWidth(next);
    }
  };

  const openMenu = () => {
    const rect = menuButtonRef.current?.getBoundingClientRect();
    setMenuAt(rect === undefined ? { x: 0, y: 0 } : { x: rect.left, y: rect.bottom + 4 });
  };

  return (
    <aside
      className={styles.panel}
      style={{ width }}
      aria-labelledby={titleId}
      data-testid={testId}
      data-width={width}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !event.defaultPrevented) {
          event.preventDefault();
          onCollapse();
        }
      }}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t('details_panel.resize')}
        aria-valuemin={DETAILS_PANEL_MIN_WIDTH}
        aria-valuemax={DETAILS_PANEL_MAX_WIDTH}
        aria-valuenow={width}
        tabIndex={0}
        className={styles.resizeHandle}
        data-dragging={dragging || undefined}
        data-testid={`${testId}-resize-handle`}
        onPointerDown={handlePointerDown}
        onKeyDown={handleResizeKeyDown}
      />
      <header className={styles.header}>
        <h2 id={titleId} className={styles.title} data-testid={`${testId}-title`}>
          {title}
        </h2>
        {hasMenu ? (
          <IconButton
            ref={menuButtonRef}
            size="sm"
            icon={<Ellipsis />}
            aria-label={t('details_panel.menu')}
            aria-haspopup="menu"
            aria-expanded={menuAt !== null}
            onClick={() => (menuAt === null ? openMenu() : setMenuAt(null))}
            data-testid={`${testId}-menu-button`}
          />
        ) : null}
        <IconButton
          size="sm"
          icon={<PanelRightClose />}
          aria-label={t('details_panel.collapse')}
          onClick={onCollapse}
          data-testid={`${testId}-collapse`}
        />
      </header>
      <div className={styles.body} data-testid={`${testId}-body`}>
        {children}
      </div>
      {footer === undefined || footer === null ? null : <footer className={styles.footer}>{footer}</footer>}
      {hasMenu && menuAt !== null ? (
        <ContextMenu
          items={menuItems}
          at={menuAt}
          anchorRef={menuButtonRef}
          onClose={() => setMenuAt(null)}
          aria-label={t('details_panel.menu')}
          data-testid={`${testId}-menu`}
        />
      ) : null}
    </aside>
  );
}

export default DetailsPanel;
