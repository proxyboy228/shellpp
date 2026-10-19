import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createDragController, type DragOptions, type DragView } from './dnd';

function card(testId: 'host-card' | 'group-card', id: string, left: number): HTMLElement {
  const element = document.createElement('div');
  element.setAttribute('data-testid', testId);
  element.setAttribute('data-record-id', id);
  element.getBoundingClientRect = () => new DOMRect(left, 0, 100, 64);
  return element;
}

function pointer(type: string, x: number, y = 30, init: MouseEventInit = {}): MouseEvent {
  return new MouseEvent(type, { clientX: x, clientY: y, button: 0, bubbles: true, ...init });
}

describe('card drag and drop (pointer events)', () => {
  let container: HTMLElement;
  let cards: Record<string, HTMLElement>;
  let options: DragOptions;
  let views: (DragView | null)[];
  let under: HTMLElement | null;

  beforeEach(() => {
    container = document.createElement('div');
    container.getBoundingClientRect = () => new DOMRect(0, 0, 800, 600);
    cards = {
      g1: card('group-card', 'g1', 0),
      g2: card('group-card', 'g2', 120),
      h1: card('host-card', 'h1', 0),
      h2: card('host-card', 'h2', 120),
      h3: card('host-card', 'h3', 240),
    };
    Object.values(cards).forEach((element) => container.append(element));
    document.body.append(container);
    views = [];
    under = null;
    options = {
      container,
      canReorder: true,
      layout: 'grid',
      draggedIds: (item) => [item.id],
      canDropInto: (type, ids, groupId) => type === 'host' || !ids.includes(groupId),
      onDrop: vi.fn(),
    };
  });

  afterEach(() => {
    container.remove();
  });

  function controller() {
    return createDragController({
      options: () => options,
      render: (view) => views.push(view),
      hitTest: () => under,
    });
  }

  it('does not start below the 5 px threshold', () => {
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    window.dispatchEvent(pointer('pointermove', 13));
    window.dispatchEvent(pointer('pointerup', 13));
    expect(views).toEqual([]);
    expect(options.onDrop).not.toHaveBeenCalled();
    expect(drag.consumeClick()).toBe(false);
    drag.dispose();
  });

  it('reorders hosts in manual mode: before or after the card under the pointer', () => {
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    under = cards.h3 ?? null;
    window.dispatchEvent(pointer('pointermove', 250));
    expect(views.at(-1)?.intent).toEqual({ kind: 'reorder', targetId: 'h3', position: 'before' });
    expect(views.at(-1)?.indicator).toMatchObject({ left: 240, orientation: 'vertical', height: 64 });
    window.dispatchEvent(pointer('pointermove', 330));
    expect(views.at(-1)?.intent).toEqual({ kind: 'reorder', targetId: 'h3', position: 'after' });
    window.dispatchEvent(pointer('pointerup', 330));
    expect(options.onDrop).toHaveBeenCalledWith({ type: 'host', id: 'h1' }, ['h1'], {
      kind: 'reorder',
      targetId: 'h3',
      position: 'after',
    });
    expect(views.at(-1)).toBeNull();
    // The click that follows the drop is swallowed once.
    expect(drag.consumeClick()).toBe(true);
    expect(drag.consumeClick()).toBe(false);
    drag.dispose();
  });

  it('does not reorder outside manual mode', () => {
    options.canReorder = false;
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    under = cards.h2 ?? null;
    window.dispatchEvent(pointer('pointermove', 150));
    window.dispatchEvent(pointer('pointerup', 150));
    expect(views[0]?.intent).toBeNull();
    expect(options.onDrop).not.toHaveBeenCalled();
    drag.dispose();
  });

  it('moves hosts into a group card in any sort mode', () => {
    options.canReorder = false;
    options.draggedIds = () => ['h1', 'h2'];
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    under = cards.g2 ?? null;
    window.dispatchEvent(pointer('pointermove', 170));
    expect(views.at(-1)).toMatchObject({ ids: ['h1', 'h2'], intent: { kind: 'into', groupId: 'g2' }, indicator: null });
    window.dispatchEvent(pointer('pointerup', 170));
    expect(options.onDrop).toHaveBeenCalledWith({ type: 'host', id: 'h1' }, ['h1', 'h2'], { kind: 'into', groupId: 'g2' });
    drag.dispose();
  });

  it('reorders groups at the card edges and moves them in at the center', () => {
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'group', id: 'g1' });
    under = cards.g2 ?? null;
    window.dispatchEvent(pointer('pointermove', 125));
    expect(views.at(-1)?.intent).toEqual({ kind: 'reorder', targetId: 'g2', position: 'before' });
    window.dispatchEvent(pointer('pointermove', 170));
    expect(views.at(-1)?.intent).toEqual({ kind: 'into', groupId: 'g2' });
    drag.dispose();
  });

  it('refuses a group dropped into itself or its subtree', () => {
    options.canDropInto = () => false;
    options.canReorder = false;
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'group', id: 'g1' });
    under = cards.g2 ?? null;
    window.dispatchEvent(pointer('pointermove', 170));
    window.dispatchEvent(pointer('pointerup', 170));
    expect(views[0]?.intent).toBeNull();
    expect(options.onDrop).not.toHaveBeenCalled();
    drag.dispose();
  });

  it('ignores the dragged card itself and presses with modifiers', () => {
    const drag = controller();
    drag.press(pointer('pointerdown', 10, 30, { ctrlKey: true }), { type: 'host', id: 'h1' });
    window.dispatchEvent(pointer('pointermove', 200));
    expect(views).toEqual([]);
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    under = cards.h1 ?? null;
    window.dispatchEvent(pointer('pointermove', 60));
    expect(views.at(-1)?.intent).toBeNull();
    drag.dispose();
  });

  it('cancels with Escape', () => {
    const drag = controller();
    drag.press(pointer('pointerdown', 10), { type: 'host', id: 'h1' });
    under = cards.h2 ?? null;
    window.dispatchEvent(pointer('pointermove', 150));
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(views.at(-1)).toBeNull();
    window.dispatchEvent(pointer('pointerup', 150));
    expect(options.onDrop).not.toHaveBeenCalled();
    drag.dispose();
  });

  it('uses the vertical position in a grid narrowed to one column', () => {
    container.style.display = 'grid';
    container.style.gridTemplateColumns = '300px';
    const drag = controller();
    drag.press(pointer('pointerdown', 10, 10), { type: 'host', id: 'h1' });
    under = cards.h2 ?? null;
    window.dispatchEvent(pointer('pointermove', 210, 10));
    expect(views.at(-1)?.intent).toEqual({ kind: 'reorder', targetId: 'h2', position: 'before' });
    expect(views.at(-1)?.indicator).toMatchObject({ orientation: 'horizontal', top: 0 });
    drag.dispose();
  });

  it('places a horizontal indicator in the list layout', () => {
    options.layout = 'list';
    const drag = controller();
    drag.press(pointer('pointerdown', 10, 10), { type: 'host', id: 'h1' });
    under = cards.h2 ?? null;
    window.dispatchEvent(pointer('pointermove', 150, 50));
    expect(views.at(-1)?.indicator).toMatchObject({ orientation: 'horizontal', top: 64, width: 100 });
    drag.dispose();
  });
});
