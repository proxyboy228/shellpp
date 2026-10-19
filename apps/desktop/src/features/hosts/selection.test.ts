import { describe, expect, it } from 'vitest';
import { arrowTarget, clickSelection, EMPTY_SELECTION, pruneSelection } from './selection';

const order = ['a', 'b', 'c', 'd', 'e'];
const plain = { toggle: false, range: false };
const ctrl = { toggle: true, range: false };
const shift = { toggle: false, range: true };

describe('clickSelection', () => {
  it('selects one card on a plain click', () => {
    expect(clickSelection({ ids: ['a', 'b'], anchor: 'a' }, 'c', plain, order)).toEqual({ ids: ['c'], anchor: 'c' });
  });

  it('toggles a card with Ctrl and keeps display order', () => {
    const one = clickSelection(EMPTY_SELECTION, 'd', ctrl, order);
    const two = clickSelection(one, 'b', ctrl, order);
    expect(two).toEqual({ ids: ['b', 'd'], anchor: 'b' });
    expect(clickSelection(two, 'd', ctrl, order)).toEqual({ ids: ['b'], anchor: 'd' });
  });

  it('selects a range from the anchor with Shift, in both directions', () => {
    const start = clickSelection(EMPTY_SELECTION, 'b', plain, order);
    expect(clickSelection(start, 'd', shift, order)).toEqual({ ids: ['b', 'c', 'd'], anchor: 'b' });
    const back = clickSelection(clickSelection(EMPTY_SELECTION, 'd', plain, order), 'a', shift, order);
    expect(back).toEqual({ ids: ['a', 'b', 'c', 'd'], anchor: 'd' });
  });

  it('keeps the anchor across Shift clicks and adds ranges with Ctrl+Shift', () => {
    const start = clickSelection(EMPTY_SELECTION, 'c', plain, order);
    const first = clickSelection(start, 'e', shift, order);
    expect(clickSelection(first, 'a', shift, order)).toEqual({ ids: ['a', 'b', 'c'], anchor: 'c' });
    const picked = clickSelection(clickSelection(EMPTY_SELECTION, 'a', plain, order), 'd', ctrl, order);
    expect(clickSelection(picked, 'e', { toggle: true, range: true }, order)).toEqual({ ids: ['a', 'd', 'e'], anchor: 'd' });
  });

  it('starts a range at the clicked card without an anchor', () => {
    expect(clickSelection(EMPTY_SELECTION, 'c', shift, order)).toEqual({ ids: ['c'], anchor: 'c' });
  });

  it('prunes ids that are no longer shown', () => {
    const current = { ids: ['a', 'x'], anchor: 'x' };
    expect(pruneSelection(current, new Set(['a']))).toEqual({ ids: ['a'], anchor: null });
    const same = { ids: ['a'], anchor: 'a' };
    expect(pruneSelection(same, new Set(['a']))).toBe(same);
  });
});

describe('arrowTarget', () => {
  it('moves by one horizontally and by a row vertically in the grid', () => {
    expect(arrowTarget(4, 'ArrowRight', 10, 3, 'grid')).toBe(5);
    expect(arrowTarget(4, 'ArrowLeft', 10, 3, 'grid')).toBe(3);
    expect(arrowTarget(4, 'ArrowDown', 10, 3, 'grid')).toBe(7);
    expect(arrowTarget(4, 'ArrowUp', 10, 3, 'grid')).toBe(1);
  });

  it('stops at the edges', () => {
    expect(arrowTarget(0, 'ArrowLeft', 10, 3, 'grid')).toBe(0);
    expect(arrowTarget(9, 'ArrowRight', 10, 3, 'grid')).toBe(9);
    expect(arrowTarget(8, 'ArrowDown', 10, 3, 'grid')).toBe(8);
    expect(arrowTarget(1, 'ArrowUp', 10, 3, 'grid')).toBe(1);
  });

  it('uses up and down only in the list', () => {
    expect(arrowTarget(2, 'ArrowDown', 5, 1, 'list')).toBe(3);
    expect(arrowTarget(2, 'ArrowUp', 5, 1, 'list')).toBe(1);
    expect(arrowTarget(2, 'ArrowRight', 5, 1, 'list')).toBe(2);
  });

  it('starts at the first card without a selection', () => {
    expect(arrowTarget(-1, 'ArrowDown', 5, 3, 'grid')).toBe(0);
    expect(arrowTarget(-1, 'ArrowDown', 0, 3, 'grid')).toBe(-1);
  });
});
