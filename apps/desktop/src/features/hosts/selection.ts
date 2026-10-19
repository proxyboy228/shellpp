// Card selection on the Hosts screen (UI 2.3, 2.4): click selects one card,
// Ctrl+click (Cmd on macOS) toggles, Shift+click selects the range from the
// anchor in display order, arrow keys move a single selection.

import type { Uuid } from '../../ipc';

export interface Selection {
  ids: readonly Uuid[];
  /** The card a Shift+click range starts from. */
  anchor: Uuid | null;
}

export const EMPTY_SELECTION: Selection = { ids: [], anchor: null };

export interface ClickModifiers {
  /** Ctrl (Cmd on macOS): toggle the card. */
  toggle: boolean;
  /** Shift: select the range from the anchor. */
  range: boolean;
}

/** Selection after a click on `id`; `order` is the display order of the selectable cards. */
export function clickSelection(current: Selection, id: Uuid, modifiers: ClickModifiers, order: readonly Uuid[]): Selection {
  if (modifiers.range) {
    const anchor = current.anchor !== null && order.includes(current.anchor) ? current.anchor : id;
    const from = order.indexOf(anchor);
    const to = order.indexOf(id);
    if (from < 0 || to < 0) {
      return { ids: [id], anchor: id };
    }
    const range = order.slice(Math.min(from, to), Math.max(from, to) + 1);
    if (modifiers.toggle) {
      const union = [...current.ids, ...range.filter((item) => !current.ids.includes(item))];
      return { ids: order.filter((item) => union.includes(item)), anchor };
    }
    return { ids: range, anchor };
  }
  if (modifiers.toggle) {
    const ids = current.ids.includes(id) ? current.ids.filter((item) => item !== id) : [...current.ids, id];
    return { ids: order.length > 0 ? order.filter((item) => ids.includes(item)) : ids, anchor: id };
  }
  return { ids: [id], anchor: id };
}

/** Drops ids that are no longer shown. */
export function pruneSelection(current: Selection, visible: ReadonlySet<Uuid>): Selection {
  const ids = current.ids.filter((id) => visible.has(id));
  const anchor = current.anchor !== null && visible.has(current.anchor) ? current.anchor : null;
  if (ids.length === current.ids.length && anchor === current.anchor) {
    return current;
  }
  return { ids, anchor };
}

export type ArrowKey = 'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown';

export function isArrowKey(key: string): key is ArrowKey {
  return key === 'ArrowLeft' || key === 'ArrowRight' || key === 'ArrowUp' || key === 'ArrowDown';
}

/**
 * The card index an arrow key moves to. Grid: left and right move by one,
 * up and down by a row of `columns`; list: up and down move by one. Moves
 * stop at the edges; with no current card the first card is chosen.
 */
export function arrowTarget(index: number, key: ArrowKey, count: number, columns: number, layout: 'grid' | 'list'): number {
  if (count === 0) {
    return -1;
  }
  if (index < 0 || index >= count) {
    return 0;
  }
  const row = layout === 'list' ? 1 : Math.max(1, columns);
  let next = index;
  switch (key) {
    case 'ArrowLeft':
      next = layout === 'list' ? index : index - 1;
      break;
    case 'ArrowRight':
      next = layout === 'list' ? index : index + 1;
      break;
    case 'ArrowUp':
      next = index - row;
      break;
    case 'ArrowDown':
      next = index + row;
      break;
  }
  return next < 0 || next >= count ? index : next;
}

/** Number of cards in the first row of a grid (cards sharing the first card's top edge). */
export function columnsOf(cards: readonly Element[]): number {
  const first = cards[0];
  if (first === undefined) {
    return 1;
  }
  const top = first.getBoundingClientRect().top;
  let columns = 0;
  for (const card of cards) {
    if (Math.abs(card.getBoundingClientRect().top - top) > 1) {
      break;
    }
    columns += 1;
  }
  return Math.max(1, columns);
}
