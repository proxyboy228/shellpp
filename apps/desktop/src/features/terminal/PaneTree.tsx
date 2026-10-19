// Split panes of a terminal tab (UI 4.7). Panes are laid out flat (absolute
// rectangles computed from the pane tree), so splitting or closing a pane
// never remounts the terminals of the other panes. Dividers are draggable
// (pointer events, at least 10% per pane); the active pane is highlighted
// when the tab has several panes. Each pane shows:
//
// - an empty pane: the host picker (search + Recent; choosing a host
//   connects in this pane, Alt opens a new window; Escape closes the pane);
// - a session that has not been connected yet, failed, or asks something:
//   the connection screen (S7's `ConnectingView`; over the terminal while a
//   reconnect asks or fails);
// - otherwise the terminal with the end overlays of UI 4.6.
//
// Prompts and host key dialogs take keyboard focus only in the active pane
// of the visible tab (`focused`), so typing in one pane never ends up in
// another pane's password field or dialog.
//
// Owner: S6.

import { useRef, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react';
import { ConnectingView } from '../connect';
import { HostPicker } from '../newtab';
import { t } from '../../i18n';
import type { SessionInfo } from '../../ipc';
import { activatePane, closePane, openSession } from './api';
import { SessionDroppedOverlay, SessionEndedOverlay, SessionReconnectingOverlay } from './SessionOverlays';
import { wasConnected } from './sessionsSync';
import { TerminalView } from './TerminalView';
import { MIN_PANE_SIZE, resizeSplit, workspaceStore, type PaneLeaf, type PaneNode, type TerminalTab } from './workspace';
import styles from './TerminalHost.module.css';

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface PaneLayout {
  leaves: { leaf: PaneLeaf; rect: Rect }[];
  dividers: {
    /** Child indexes from the root to the split. */
    path: number[];
    /** The divider sits after child `index`. */
    index: number;
    direction: 'row' | 'column';
    sizes: number[];
    /** The split's rectangle. */
    rect: Rect;
    /** Position of the divider along the split's axis (0..1 of the split). */
    at: number;
  }[];
}

/** Rectangles (fractions of the tab area) of every pane and divider of a tree. */
function layoutPanes(root: PaneNode | null): PaneLayout {
  const layout: PaneLayout = { leaves: [], dividers: [] };
  const visit = (node: PaneNode, rect: Rect, path: number[]) => {
    if (node.type === 'leaf') {
      layout.leaves.push({ leaf: node, rect });
      return;
    }
    const total = node.sizes.reduce((sum, size) => sum + size, 0) || 1;
    let offset = 0;
    node.children.forEach((child, index) => {
      const share = (node.sizes[index] ?? 0) / total;
      const childRect =
        node.direction === 'row'
          ? { x: rect.x + offset * rect.w, y: rect.y, w: share * rect.w, h: rect.h }
          : { x: rect.x, y: rect.y + offset * rect.h, w: rect.w, h: share * rect.h };
      visit(child, childRect, [...path, index]);
      offset += share;
      if (index < node.children.length - 1) {
        layout.dividers.push({ path, index, direction: node.direction, sizes: node.sizes, rect, at: offset });
      }
    });
  };
  if (root !== null) {
    visit(root, { x: 0, y: 0, w: 1, h: 1 }, []);
  }
  return layout;
}

function percent(value: number): string {
  return `${(value * 100).toFixed(4)}%`;
}

function rectStyle(rect: Rect): CSSProperties {
  return { left: percent(rect.x), top: percent(rect.y), width: percent(rect.w), height: percent(rect.h) };
}

interface PaneContentProps {
  tab: TerminalTab;
  leaf: PaneLeaf;
  session: SessionInfo | undefined;
  focused: boolean;
}

function PaneContent({ tab, leaf, session, focused }: PaneContentProps) {
  const close = () => closePane(leaf.paneId);
  if (leaf.sessionId === null) {
    return (
      <div className={styles.picker}>
        <HostPicker
          testIdPrefix="pane-host-picker"
          autoFocus={focused}
          active={focused}
          includeTabs={false}
          onPickHost={(target, event) =>
            void openSession(target, event.altKey ? { newWindow: true } : { inPane: { tabId: tab.id, paneId: leaf.paneId } })
          }
          onCancel={close}
        />
      </div>
    );
  }
  if (session === undefined) {
    return null;
  }
  const connectedBefore = wasConnected(session.session_id);
  const asks = session.state === 'failed' || session.prompt !== undefined;
  if (!connectedBefore && (asks || session.state === 'connecting')) {
    return <ConnectingView session={session} onClose={close} focused={focused} />;
  }
  return (
    <>
      <TerminalView session={session} paneId={leaf.paneId} focused={focused} />
      {session.state === 'ended' ? <SessionEndedOverlay session={session} onClose={close} /> : null}
      {session.state === 'dropped' ? <SessionDroppedOverlay session={session} onClose={close} /> : null}
      {asks ? (
        <div className={styles.screenOverlay}>
          <ConnectingView session={session} onClose={close} focused={focused} />
        </div>
      ) : null}
      {session.state === 'connecting' && !asks ? <SessionReconnectingOverlay session={session} onClose={close} /> : null}
    </>
  );
}

interface DividerProps {
  tab: TerminalTab;
  divider: PaneLayout['dividers'][number];
  areaRef: { readonly current: HTMLDivElement | null };
}

function Divider({ tab, divider, areaRef }: DividerProps) {
  const { path, index, direction, sizes, rect, at } = divider;

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    const area = areaRef.current;
    const element = event.currentTarget;
    if (area === null || event.button !== 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    element.setPointerCapture?.(event.pointerId);
    const bounds = area.getBoundingClientRect();
    const splitPixels = direction === 'row' ? bounds.width * rect.w : bounds.height * rect.h;
    const start = direction === 'row' ? event.clientX : event.clientY;
    const total = sizes.reduce((sum, size) => sum + size, 0) || 1;
    const initial = sizes.map((size) => size / total);
    const pair = (initial[index] ?? 0) + (initial[index + 1] ?? 0);
    const onMove = (move: PointerEvent) => {
      if (!(splitPixels > 0)) {
        return;
      }
      const delta = ((direction === 'row' ? move.clientX : move.clientY) - start) / splitPixels;
      const first = Math.min(pair - MIN_PANE_SIZE, Math.max(MIN_PANE_SIZE, (initial[index] ?? 0) + delta));
      const next = [...initial];
      next[index] = first;
      next[index + 1] = pair - first;
      workspaceStore.set((current) => resizeSplit(current, tab.id, path, next));
    };
    const onUp = () => {
      element.removeEventListener('pointermove', onMove);
      element.removeEventListener('pointerup', onUp);
      element.removeEventListener('pointercancel', onUp);
    };
    element.addEventListener('pointermove', onMove);
    element.addEventListener('pointerup', onUp);
    element.addEventListener('pointercancel', onUp);
  };

  const style: CSSProperties =
    direction === 'row'
      ? { left: percent(rect.x + at * rect.w), top: percent(rect.y), height: percent(rect.h) }
      : { top: percent(rect.y + at * rect.h), left: percent(rect.x), width: percent(rect.w) };

  return (
    <div
      className={styles.divider}
      style={style}
      role="separator"
      aria-orientation={direction === 'row' ? 'vertical' : 'horizontal'}
      aria-label={t('splits.divider')}
      data-testid="split-divider"
      data-direction={direction}
      data-index={index}
      onPointerDown={onPointerDown}
    />
  );
}

export interface PaneTreeProps {
  tab: TerminalTab;
  sessions: ReadonlyMap<string, SessionInfo>;
  /** The tab is the active tab and the terminal area is visible. */
  tabVisible: boolean;
}

export function PaneTree({ tab, sessions, tabVisible }: PaneTreeProps) {
  const areaRef = useRef<HTMLDivElement>(null);
  const layout = layoutPanes(tab.root);
  const multiple = layout.leaves.length > 1;
  return (
    <div ref={areaRef} className={styles.paneArea}>
      {layout.leaves.map(({ leaf, rect }) => {
        const active = tab.activePaneId === leaf.paneId;
        const session = leaf.sessionId === null ? undefined : sessions.get(leaf.sessionId);
        return (
          <div
            key={leaf.paneId}
            className={styles.pane}
            style={rectStyle(rect)}
            role="group"
            aria-label={t('terminal.pane')}
            data-testid="terminal-pane"
            data-pane-id={leaf.paneId}
            data-session-id={leaf.sessionId ?? undefined}
            data-active={active}
            data-highlight={multiple && active ? 'true' : undefined}
            data-state={session?.state}
            onPointerDown={() => activatePane(leaf.paneId)}
          >
            <PaneContent tab={tab} leaf={leaf} session={session} focused={tabVisible && active} />
          </div>
        );
      })}
      {layout.dividers.map((divider) => (
        <Divider key={`${divider.path.join('.')}:${divider.index}`} tab={tab} divider={divider} areaRef={areaRef} />
      ))}
    </div>
  );
}
