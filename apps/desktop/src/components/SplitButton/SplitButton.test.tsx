import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { FolderPlus, Plus } from 'lucide-react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { SplitButton } from './SplitButton';
import type { SplitButtonItem } from './SplitButton';
import css from './SplitButton.module.css?raw';
import styles from './SplitButton.module.css';

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

function makeItems() {
  const onGroup = vi.fn();
  const onImport = vi.fn();
  const onArchived = vi.fn();
  const onRemove = vi.fn();
  const items: SplitButtonItem[] = [
    { id: 'group', label: 'New Group', icon: <FolderPlus />, onSelect: onGroup },
    { id: 'import', label: 'Import', onSelect: onImport },
    { id: 'archived', label: 'Archived', disabled: true, onSelect: onArchived },
    { id: 'remove', label: 'Remove all', danger: true, onSelect: onRemove },
  ];
  return { items, onGroup, onImport, onArchived, onRemove };
}

function renderSplit(props: Partial<Parameters<typeof SplitButton>[0]> = {}) {
  const { items, ...handlers } = makeItems();
  const onClick = vi.fn();
  const view = render(
    <>
      <SplitButton items={items} onClick={onClick} icon={<Plus />} data-testid="new-host" {...props}>
        New host
      </SplitButton>
      <button type="button">Outside</button>
    </>,
  );
  const main = screen.getByRole('button', { name: 'New host' });
  const toggle = screen.getByRole('button', { name: 'More actions' });
  return { ...view, ...handlers, onClick, main, toggle };
}

describe('SplitButton', () => {
  it('renders the main action and a closed menu button', () => {
    const { main, toggle } = renderSplit();

    expect(main).toHaveAttribute('type', 'button');
    expect(toggle).toHaveAttribute('aria-haspopup', 'menu');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(toggle).not.toHaveAttribute('aria-controls');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(screen.getByTestId('new-host')).toHaveAttribute('data-variant', 'primary');
    expect(screen.getByTestId('new-host-main')).toBe(main);
    expect(screen.getByTestId('new-host-toggle')).toBe(toggle);
  });

  it('runs the main action without opening the menu', async () => {
    const user = userEvent.setup();
    const { main, onClick } = renderSplit();

    await user.click(main);
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('opens the menu on click and focuses the first item', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);

    const menu = screen.getByRole('menu');
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(toggle).toHaveAttribute('aria-controls', menu.id);
    expect(menu).toHaveAttribute('aria-labelledby', toggle.id);
    expect(screen.getByTestId('new-host-menu')).toBe(menu);

    const items = screen.getAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual(['New Group', 'Import', 'Archived', 'Remove all']);
    expect(items[0]).toHaveFocus();
    // Focus is managed by the menu; items are never in the tab order.
    for (const item of items) {
      expect(item).toHaveAttribute('tabindex', '-1');
    }
    expect(items[2]).toHaveAttribute('aria-disabled', 'true');
    expect(items[3]).toHaveClass(styles.danger);
    expect(screen.getByTestId('new-host-item-import')).toBe(items[1]);
  });

  it('closes when the arrow is clicked again', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.click(toggle);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
  });

  it('opens from the keyboard: Enter and ArrowDown focus the first item, ArrowUp the last', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    toggle.focus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('menuitem', { name: 'New Group' })).toHaveFocus();
    await user.keyboard('{Escape}');

    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('menuitem', { name: 'New Group' })).toHaveFocus();
    await user.keyboard('{Escape}');

    await user.keyboard('{ArrowUp}');
    expect(screen.getByRole('menuitem', { name: 'Remove all' })).toHaveFocus();
  });

  it('moves focus with arrows, Home and End, skips disabled items and wraps', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    const item = (name: string) => screen.getByRole('menuitem', { name });

    await user.keyboard('{ArrowDown}');
    expect(item('Import')).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(item('Remove all')).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(item('New Group')).toHaveFocus();
    await user.keyboard('{ArrowUp}');
    expect(item('Remove all')).toHaveFocus();
    await user.keyboard('{Home}');
    expect(item('New Group')).toHaveFocus();
    await user.keyboard('{End}');
    expect(item('Remove all')).toHaveFocus();
  });

  it('moves to an item by its first letter', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.keyboard('i');
    expect(screen.getByRole('menuitem', { name: 'Import' })).toHaveFocus();
    await user.keyboard('r');
    expect(screen.getByRole('menuitem', { name: 'Remove all' })).toHaveFocus();
  });

  it('selects an item with Enter, closes and returns focus to the arrow', async () => {
    const user = userEvent.setup();
    const { toggle, onImport, onClick } = renderSplit();

    await user.click(toggle);
    await user.keyboard('{ArrowDown}{Enter}');

    expect(onImport).toHaveBeenCalledTimes(1);
    expect(onClick).not.toHaveBeenCalled();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(toggle).toHaveFocus();
  });

  it('selects an item with Space and with a click', async () => {
    const user = userEvent.setup();
    const { toggle, onGroup, onRemove } = renderSplit();

    await user.click(toggle);
    await user.keyboard('[Space]');
    expect(onGroup).toHaveBeenCalledTimes(1);

    await user.click(toggle);
    await user.click(screen.getByRole('menuitem', { name: 'Remove all' }));
    expect(onRemove).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('does not select a disabled item', async () => {
    const user = userEvent.setup();
    const { toggle, onArchived } = renderSplit();

    await user.click(toggle);
    await user.click(screen.getByRole('menuitem', { name: 'Archived' }));
    expect(onArchived).not.toHaveBeenCalled();
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('closes on Escape and returns focus to the arrow', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(toggle).toHaveFocus();
  });

  it('closes on a click outside', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.click(document.body);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
  });

  it('closes on Tab and moves on to the next control', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.keyboard('{ArrowDown}');
    await user.tab();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Outside' })).toHaveFocus();
  });

  it('closes on Shift+Tab and returns focus to the arrow', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    await user.tab({ shift: true });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(toggle).toHaveFocus();
  });

  it('closes when focus moves outside by other means', async () => {
    const user = userEvent.setup();
    const { toggle } = renderSplit();

    await user.click(toggle);
    act(() => {
      screen.getByRole('button', { name: 'Outside' }).focus();
    });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('disables both parts', async () => {
    const user = userEvent.setup();
    const { main, toggle, onClick } = renderSplit({ disabled: true });

    expect(main).toBeDisabled();
    expect(toggle).toBeDisabled();
    await user.click(main);
    await user.click(toggle);
    expect(onClick).not.toHaveBeenCalled();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('supports the secondary variant, a custom menu label and end alignment', async () => {
    const user = userEvent.setup();
    const { items } = makeItems();
    render(
      <SplitButton items={items} variant="secondary" menuLabel="Key options" menuAlign="end" data-testid="new-key">
        New key
      </SplitButton>,
    );

    expect(screen.getByTestId('new-key')).toHaveAttribute('data-variant', 'secondary');
    await user.click(screen.getByRole('button', { name: 'Key options' }));
    expect(screen.getByRole('menu')).toHaveClass(styles.alignEnd);
  });

  it('uses the item test id when one is given', async () => {
    const user = userEvent.setup();
    render(
      <SplitButton items={[{ id: 'a', label: 'Alpha', onSelect: () => undefined, 'data-testid': 'alpha-item' }]}>
        Main
      </SplitButton>,
    );

    await user.click(screen.getByRole('button', { name: 'More actions' }));
    expect(screen.getByTestId('alpha-item')).toHaveAccessibleName('Alpha');
  });
});

describe('SplitButton styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('styles menu states and reduced motion per the design system', () => {
    expect(css).toMatch(
      /\.item:focus-visible\s*\{\s*outline: var\(--focus-ring-width\) solid var\(--focus-ring\);\s*outline-offset: var\(--focus-ring-offset\);/,
    );
    expect(css).toContain('background-color: var(--bg-overlay-hover)');
    expect(css).toContain('color: var(--text-disabled)');
    expect(css).toContain('color: var(--danger-hover)');
    expect(css).toContain('animation: menu-in var(--duration-normal) var(--easing-standard)');
    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
  });
});
