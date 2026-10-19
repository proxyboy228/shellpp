import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Copy, Pencil, Trash2 } from 'lucide-react';
import { useRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Menu, type MenuEntry } from './Menu';
import css from './Menu.module.css?raw';
import styles from './Menu.module.css';

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]).filter((name) => !defined.has(name));
}

function hostMenu(overrides: Partial<Record<string, () => void>> = {}): MenuEntry[] {
  return [
    { id: 'connect', label: 'Connect', submenu: true, onSelect: overrides.connect, 'data-testid': 'menu-connect' },
    { id: 'edit', label: 'Edit', icon: Pencil, shortcut: 'E', onSelect: overrides.edit, 'data-testid': 'menu-edit' },
    { id: 'duplicate', label: 'Duplicate', icon: Copy, disabled: true, onSelect: overrides.duplicate },
    { type: 'separator', id: 'sep-1' },
    { id: 'remove', label: 'Remove', icon: Trash2, danger: true, onSelect: overrides.remove },
  ];
}

describe('Menu', () => {
  it('renders a labelled menu with items and separators', () => {
    render(<Menu items={hostMenu()} aria-label="Host actions" data-testid="menu" />);

    const menu = screen.getByRole('menu', { name: 'Host actions' });
    expect(menu).toBe(screen.getByTestId('menu'));
    expect(menu).toHaveClass(styles.menu);
    expect(screen.getAllByRole('menuitem')).toHaveLength(4);
    expect(screen.getAllByRole('separator')).toHaveLength(1);
    expect(screen.getByTestId('menu-edit')).toHaveAccessibleName('Edit');
  });

  it('shows shortcut hints, danger, disabled and submenu items', () => {
    render(<Menu items={hostMenu()} aria-label="Host actions" />);

    const edit = screen.getByRole('menuitem', { name: 'Edit' });
    expect(edit).toHaveAttribute('aria-keyshortcuts', 'E');
    expect(edit).toHaveTextContent('E');

    const remove = screen.getByRole('menuitem', { name: 'Remove' });
    expect(remove).toHaveClass(styles.danger);
    expect(remove).toHaveAttribute('data-danger', 'true');

    const duplicate = screen.getByRole('menuitem', { name: 'Duplicate' });
    expect(duplicate).toHaveAttribute('aria-disabled', 'true');
    expect(duplicate).toHaveClass(styles.itemDisabled);

    const connect = screen.getByRole('menuitem', { name: 'Connect' });
    expect(connect).toHaveAttribute('aria-haspopup', 'menu');
    expect(screen.getByTestId('menu-connect-submenu')).toHaveClass('lucide-chevron-right');
    expect(edit).not.toHaveAttribute('aria-haspopup');
  });

  it('focuses the first enabled item on mount', () => {
    const items: MenuEntry[] = [
      { id: 'a', label: 'Alpha', disabled: true },
      { id: 'b', label: 'Beta' },
    ];
    render(<Menu items={items} aria-label="Actions" />);

    expect(screen.getByRole('menuitem', { name: 'Beta' })).toHaveFocus();
  });

  it('can leave focus where it is', () => {
    render(<Menu items={hostMenu()} aria-label="Host actions" autoFocus={false} />);

    expect(document.body).toHaveFocus();
  });

  it('moves with Up, Down, Home and End, skipping disabled items and separators', async () => {
    const user = userEvent.setup();
    render(<Menu items={hostMenu()} aria-label="Host actions" />);
    const item = (name: string) => screen.getByRole('menuitem', { name });

    expect(item('Connect')).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(item('Edit')).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(item('Remove')).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(item('Connect')).toHaveFocus();
    await user.keyboard('{ArrowUp}');
    expect(item('Remove')).toHaveFocus();
    await user.keyboard('{ArrowUp}');
    expect(item('Edit')).toHaveFocus();
    await user.keyboard('{End}');
    expect(item('Remove')).toHaveFocus();
    await user.keyboard('{Home}');
    expect(item('Connect')).toHaveFocus();
  });

  it('moves to the next item starting with a typed letter', async () => {
    const user = userEvent.setup();
    render(<Menu items={hostMenu()} aria-label="Host actions" />);

    await user.keyboard('r');
    expect(screen.getByRole('menuitem', { name: 'Remove' })).toHaveFocus();
    await user.keyboard('d');
    expect(screen.getByRole('menuitem', { name: 'Remove' })).toHaveFocus();
  });

  it('activates an item with Enter, Space or a click and then closes', async () => {
    const user = userEvent.setup();
    const edit = vi.fn();
    const remove = vi.fn();
    const onAction = vi.fn();
    const onClose = vi.fn();
    render(
      <Menu items={hostMenu({ edit, remove })} aria-label="Host actions" onAction={onAction} onClose={onClose} />,
    );

    await user.keyboard('{ArrowDown}{Enter}');
    expect(edit).toHaveBeenCalledTimes(1);
    expect(onAction).toHaveBeenLastCalledWith('edit');
    expect(onClose).toHaveBeenCalledTimes(1);

    await user.keyboard(' ');
    expect(edit).toHaveBeenCalledTimes(2);
    expect(onClose).toHaveBeenCalledTimes(2);

    await user.click(screen.getByRole('menuitem', { name: 'Remove' }));
    expect(remove).toHaveBeenCalledTimes(1);
    expect(onAction).toHaveBeenLastCalledWith('remove');
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it('keeps the menu open when a submenu item is activated', async () => {
    const user = userEvent.setup();
    const connect = vi.fn();
    const onAction = vi.fn();
    const onClose = vi.fn();
    render(
      <Menu items={hostMenu({ connect })} aria-label="Host actions" onAction={onAction} onClose={onClose} />,
    );

    await user.keyboard('{ArrowRight}');
    await user.keyboard('{Enter}');
    expect(connect).toHaveBeenCalledTimes(2);
    expect(onAction).toHaveBeenCalledWith('connect');
    expect(onClose).not.toHaveBeenCalled();

    // Right does nothing on an item without a submenu.
    await user.keyboard('{ArrowDown}{ArrowRight}');
    expect(onAction).toHaveBeenCalledTimes(2);
  });

  it('does not activate or focus disabled items', async () => {
    const user = userEvent.setup();
    const duplicate = vi.fn();
    const onAction = vi.fn();
    const onClose = vi.fn();
    render(
      <Menu items={hostMenu({ duplicate })} aria-label="Host actions" onAction={onAction} onClose={onClose} />,
    );

    const item = screen.getByRole('menuitem', { name: 'Duplicate' });
    await user.hover(item);
    expect(item).not.toHaveFocus();
    await user.click(item);
    expect(item).not.toHaveFocus();
    expect(duplicate).not.toHaveBeenCalled();
    expect(onAction).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('focuses an item under the pointer', async () => {
    const user = userEvent.setup();
    render(<Menu items={hostMenu()} aria-label="Host actions" />);

    await user.hover(screen.getByRole('menuitem', { name: 'Remove' }));
    expect(screen.getByRole('menuitem', { name: 'Remove' })).toHaveFocus();
  });

  it('closes on Escape without letting the key reach its ancestors', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const onParentKeyDown = vi.fn();
    render(
      <div onKeyDown={onParentKeyDown}>
        <Menu items={hostMenu()} aria-label="Host actions" onClose={onClose} />
      </div>,
    );

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onParentKeyDown).not.toHaveBeenCalled();
  });

  it('closes on Tab', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Menu items={hostMenu()} aria-label="Host actions" onClose={onClose} />);

    await user.tab();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on a press outside the menu and its anchor', () => {
    const onClose = vi.fn();
    function Harness() {
      const anchorRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button type="button" ref={anchorRef}>
            Trigger
          </button>
          <p>Elsewhere</p>
          <Menu items={hostMenu()} aria-label="Host actions" onClose={onClose} anchorRef={anchorRef} />
        </>
      );
    }
    render(<Harness />);

    fireEvent.pointerDown(screen.getByRole('menuitem', { name: 'Edit' }));
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Trigger' }));
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.pointerDown(screen.getByText('Elsewhere'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('returns focus to the trigger when it closes from the keyboard', async () => {
    const user = userEvent.setup();
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Actions
          </button>
          {open ? <Menu items={hostMenu()} aria-label="Host actions" onClose={() => setOpen(false)} /> : null}
        </>
      );
    }
    render(<Harness />);

    const trigger = screen.getByRole('button', { name: 'Actions' });
    await user.click(trigger);
    expect(screen.getByRole('menuitem', { name: 'Connect' })).toHaveFocus();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it('uses only design tokens and follows the state rules', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    expect(undefinedTokens(css)).toEqual([]);
    expect(css).toMatch(/\.menu\s*\{[^}]*border-radius:\s*var\(--radius-overlay\)/);
    expect(css).toMatch(/\.menu\s*\{[^}]*background:\s*var\(--bg-overlay\)/);
    expect(css).toMatch(/\.menu\s*\{[^}]*box-shadow:\s*var\(--shadow-overlay\)/);
    expect(css).toMatch(/:hover,[^{]*:focus\s*\{\s*background:\s*var\(--bg-overlay-hover\)/);
    expect(css).toMatch(/:active\s*\{\s*background:\s*var\(--bg-pressed\)/);
    expect(css).toMatch(/\.danger\s*\{\s*color:\s*var\(--danger\);/);
    expect(css).toMatch(/:active\s*\{\s*color:\s*var\(--danger-hover\)/);
    expect(css).toMatch(/\.itemDisabled\s*\{\s*color:\s*var\(--text-disabled\);/);
    expect(css).not.toMatch(/--text-muted/);
    expect(css).toMatch(/\.separator\s*\{[^}]*background:\s*var\(--border-strong\)/);
    expect(css).toMatch(/:focus-visible\s*\{[^}]*outline:\s*var\(--focus-ring-width\) solid var\(--focus-ring\)/);
    expect(css).toMatch(/animation:\s*menu-in var\(--duration-normal\) var\(--easing-standard\)/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.menu\s*\{\s*animation:\s*none/);
  });
});

describe('Menu info icon', () => {
  it('describes the item and shows the text on hover of the icon or keyboard focus', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <Menu
        items={[
          { id: 'edit', label: 'Edit', 'data-testid': 'menu-edit' },
          { id: 'link', label: 'Copy Link', info: 'Works only in this app.', onSelect, 'data-testid': 'menu-link' },
        ]}
        aria-label="Host actions"
      />,
    );
    const item = screen.getByTestId('menu-link');
    expect(item).toHaveAccessibleName('Copy Link');
    expect(item).toHaveAccessibleDescription('Works only in this app.');
    const text = screen.getByTestId('menu-link-info-text');
    expect(text).not.toBeVisible();

    await user.hover(screen.getByTestId('menu-link-info'));
    expect(text).toBeVisible();
    await user.click(screen.getByTestId('menu-link-info'));
    expect(onSelect).not.toHaveBeenCalled();
    await user.unhover(screen.getByTestId('menu-link-info'));
    expect(text).not.toBeVisible();

    await user.keyboard('{Home}{End}');
    expect(item).toHaveFocus();
    expect(text).toBeVisible();
  });

  it('accepts several anchor elements for outside presses', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    function Harness() {
      const first = useRef<HTMLButtonElement>(null);
      const second = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button ref={first} type="button">
            First
          </button>
          <button ref={second} type="button">
            Second
          </button>
          <button type="button">Outside</button>
          <Menu items={[{ id: 'a', label: 'A' }]} onClose={onClose} anchorRef={[first, second]} />
        </>
      );
    }
    render(<Harness />);
    await user.click(screen.getByRole('button', { name: 'First' }));
    await user.click(screen.getByRole('button', { name: 'Second' }));
    expect(onClose).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Outside' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
