// Card drag and drop on the Hosts screen (UI 2.3), built on pointer events
// so WebDriver actions can drive it (stage plan decision 13).
//
// - A drag starts after the pointer moved 5 px with the primary button held.
// - Manual sort: dropping between cards of the same section reorders
//   (before/after the card under the pointer).
// - Dropping a host or a group onto a group card moves it into that group,
//   in any sort mode; a group cannot go into itself or its subtree.
// - Escape cancels.

import type { Uuid } from '../../ipc';

export const DRAG_THRESHOLD_PX = 5;

export type CardType = 'host' | 'group';

export interface DragItem {
  type: CardType;
  id: Uuid;
}

export type DropIntent =
  | { kind: 'into'; groupId: Uuid }
  | { kind: 'reorder'; targetId: Uuid; position: 'before' | 'after' };

export interface DropIndicator {
  /** Position relative to the container's content box (scroll included). */
  left: number;
  top: number;
  width: number;
  height: number;
  orientation: 'vertical' | 'horizontal';
}

export interface DragView {
  /** Records being dragged. */
  ids: readonly Uuid[];
  intent: DropIntent | null;
  indicator: DropIndicator | null;
}

export interface DragOptions {
  container: HTMLElement | null;
  /** Reordering is allowed (Manual sort, no filter). */
  canReorder: boolean;
  layout: 'grid' | 'list';
  /** Records a drag of `item` carries (the selection when the card is selected). */
  draggedIds: (item: DragItem) => Uuid[];
  /** False when the records cannot go into the group (itself or its subtree). */
  canDropInto: (type: CardType, ids: readonly Uuid[], groupId: Uuid) => boolean;
  onDrop: (item: DragItem, ids: readonly Uuid[], intent: DropIntent) => void;
}

export interface DragControllerDeps {
  options: () => DragOptions;
  render: (view: DragView | null) => void;
  /** The element under a viewport point (default `document.elementFromPoint`). */
  hitTest?: (x: number, y: number) => Element | null;
  target?: Window;
}

export interface DragController {
  /** Call from the card's `pointerdown`. */
  press: (event: PointerEvent | MouseEvent, item: DragItem) => void;
  /** True once for the click that ends a drag; the card then ignores it. */
  consumeClick: () => boolean;
  dispose: () => void;
}

interface Session {
  item: DragItem;
  startX: number;
  startY: number;
  active: boolean;
  ids: Uuid[];
  intent: DropIntent | null;
}

const CARD_SELECTOR = '[data-testid="group-card"], [data-testid="host-card"]';

function defaultHitTest(x: number, y: number): Element | null {
  return typeof document.elementFromPoint === 'function' ? document.elementFromPoint(x, y) : null;
}

export function createDragController({ options, render, hitTest = defaultHitTest, target = window }: DragControllerDeps): DragController {
  let session: Session | null = null;
  let suppressClick = false;
  let clearSuppress: ReturnType<typeof setTimeout> | undefined;

  /** Cards follow each other vertically: the list layout, or a grid narrowed to one column. */
  function stackedVertically(card: Element): boolean {
    if (options().layout === 'list') {
      return true;
    }
    const parent = card.parentElement;
    if (parent === null) {
      return false;
    }
    const columns = getComputedStyle(parent)
      .gridTemplateColumns.trim()
      .split(/\s+/)
      .filter((track) => track !== '' && track !== 'none');
    return columns.length === 1;
  }

  function intentAt(x: number, y: number, current: Session): { intent: DropIntent; card: Element } | null {
    const opts = options();
    const card = hitTest(x, y)?.closest(CARD_SELECTOR) ?? null;
    if (card === null || opts.container === null || !opts.container.contains(card)) {
      return null;
    }
    const targetId = card.getAttribute('data-record-id');
    if (targetId === null || current.ids.includes(targetId)) {
      return null;
    }
    const targetType: CardType = card.getAttribute('data-testid') === 'group-card' ? 'group' : 'host';
    const rect = card.getBoundingClientRect();
    const vertical = stackedVertically(card);
    const extent = vertical ? rect.height : rect.width;
    const offset = vertical ? y - rect.top : x - rect.left;
    const ratio = extent > 0 ? offset / extent : 0.5;
    if (targetType === 'group') {
      if (current.item.type === 'group' && opts.canReorder && (ratio < 0.25 || ratio > 0.75)) {
        return { intent: { kind: 'reorder', targetId, position: ratio < 0.25 ? 'before' : 'after' }, card };
      }
      if (opts.canDropInto(current.item.type, current.ids, targetId)) {
        return { intent: { kind: 'into', groupId: targetId }, card };
      }
      return null;
    }
    if (current.item.type === 'host' && opts.canReorder) {
      return { intent: { kind: 'reorder', targetId, position: ratio < 0.5 ? 'before' : 'after' }, card };
    }
    return null;
  }

  function indicatorFor(intent: DropIntent, card: Element): DropIndicator | null {
    const opts = options();
    if (intent.kind !== 'reorder' || opts.container === null) {
      return null;
    }
    const box = opts.container.getBoundingClientRect();
    const rect = card.getBoundingClientRect();
    const left = rect.left - box.left + opts.container.scrollLeft;
    const top = rect.top - box.top + opts.container.scrollTop;
    if (stackedVertically(card)) {
      return {
        left,
        top: intent.position === 'before' ? top : top + rect.height,
        width: rect.width,
        height: 0,
        orientation: 'horizontal',
      };
    }
    return {
      left: intent.position === 'before' ? left : left + rect.width,
      top,
      width: 0,
      height: rect.height,
      orientation: 'vertical',
    };
  }

  function stop(): void {
    target.removeEventListener('pointermove', handleMove);
    target.removeEventListener('pointerup', handleUp);
    target.removeEventListener('pointercancel', handleCancel);
    target.removeEventListener('keydown', handleKey, true);
    const wasActive = session?.active === true;
    session = null;
    if (wasActive) {
      render(null);
    }
  }

  function handleMove(event: PointerEvent | MouseEvent): void {
    const current = session;
    if (current === null) {
      return;
    }
    if (!current.active) {
      const distance = Math.hypot(event.clientX - current.startX, event.clientY - current.startY);
      if (distance < DRAG_THRESHOLD_PX) {
        return;
      }
      current.active = true;
      current.ids = options().draggedIds(current.item);
    }
    event.preventDefault();
    const hit = intentAt(event.clientX, event.clientY, current);
    current.intent = hit?.intent ?? null;
    render({
      ids: current.ids,
      intent: current.intent,
      indicator: hit === null ? null : indicatorFor(hit.intent, hit.card),
    });
  }

  function handleUp(event: PointerEvent | MouseEvent): void {
    const current = session;
    if (current === null) {
      return;
    }
    if (current.active) {
      const hit = intentAt(event.clientX, event.clientY, current);
      suppressClick = true;
      clearTimeout(clearSuppress);
      clearSuppress = setTimeout(() => {
        suppressClick = false;
      }, 0);
      stop();
      if (hit !== null) {
        options().onDrop(current.item, current.ids, hit.intent);
      }
      return;
    }
    stop();
  }

  function handleCancel(): void {
    stop();
  }

  function handleKey(event: KeyboardEvent): void {
    if (event.key === 'Escape' && session?.active === true) {
      event.preventDefault();
      event.stopPropagation();
      stop();
    }
  }

  return {
    press(event, item) {
      if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) {
        return;
      }
      stop();
      session = { item, startX: event.clientX, startY: event.clientY, active: false, ids: [], intent: null };
      target.addEventListener('pointermove', handleMove);
      target.addEventListener('pointerup', handleUp);
      target.addEventListener('pointercancel', handleCancel);
      target.addEventListener('keydown', handleKey, true);
    },
    consumeClick() {
      if (suppressClick) {
        suppressClick = false;
        return true;
      }
      return false;
    },
    dispose() {
      clearTimeout(clearSuppress);
      stop();
    },
  };
}
