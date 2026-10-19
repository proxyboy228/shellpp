// ContextMenu (W4/W5 contract): submenus, info icon, actions, clamping, focus.

import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef, useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ContextMenu, type ContextMenuEntry } from './ContextMenu';

afterEach(() => {
  vi.restoreAllMocks();
});

function hostMenu(onSelect: (id: string) => void): ContextMenuEntry[] {
  return [
    { id: 'edit', label: 'Edit Host Details', shortcut: 'E', 'data-testid': 'host-menu-edit', onSelect: () => onSelect('edit') },
    {
      id: 'move-to',
      label: 'Move to',
      'data-testid': 'host-menu-move-to',
      children: [
        { id: 'top', label: 'Top level', 'data-testid': 'host-menu-move-to-top-level', onSelect: () => onSelect('top') },
        { id: 'group', label: 'Production', 'data-testid': 'host-menu-move-to-group-1', onSelect: () => onSelect('group') },
      ],
    },
    {
      id: 'copy-link',
      label: 'Copy Link',
      info: 'Opens this host in Shell++ for someone who has it in a vault on their device.',
      'data-testid': 'host-menu-copy-link',
      onSelect: () => onSelect('copy-link'),
    },
    { type: 'separator', id: 'sep' },
    { id: 'remove', label: 'Remove', danger: true, 'data-testid': 'host-menu-remove', onSelect: () => onSelect('remove') },
  ];
}

function Harness({ onSelect, onAction }: { onSelect: (id: string) => void; onAction?: (id: string) => void }) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={buttonRef} type="button" onClick={() => setOpen((value) => !value)}>
        Open
      </button>
      <button type="button">Elsewhere</button>
      {open ? (
        <ContextMenu
          items={hostMenu(onSelect)}
          at={{ x: 10, y: 20 }}
          anchorRef={buttonRef}
          onClose={() => setOpen(false)}
          onAction={onAction}
          data-testid="host-context-menu"
        />
      ) : null}
    </>
  );
}

describe('ContextMenu', () => {
  it('runs a leaf item, reports the action and closes, returning focus', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onAction = vi.fn();
    render(<Harness onSelect={onSelect} onAction={onAction} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    expect(screen.getByTestId('host-context-menu')).toBeInTheDocument();
    expect(screen.getByTestId('host-menu-edit')).toHaveFocus();
    await user.click(screen.getByTestId('host-menu-remove'));
    expect(onSelect).toHaveBeenCalledWith('remove');
    expect(onAction).toHaveBeenCalledWith('remove');
    expect(screen.queryByTestId('host-context-menu')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toHaveFocus();
  });

  it('opens a submenu on hover and runs its items', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onAction = vi.fn();
    render(<Harness onSelect={onSelect} onAction={onAction} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.hover(screen.getByTestId('host-menu-move-to'));
    const submenu = screen.getByTestId('host-menu-move-to-menu');
    expect(screen.getByTestId('host-menu-move-to')).toHaveAttribute('aria-haspopup', 'menu');
    await user.hover(screen.getByTestId('host-menu-edit'));
    expect(screen.queryByTestId('host-menu-move-to-menu')).not.toBeInTheDocument();
    await user.click(screen.getByTestId('host-menu-move-to'));
    expect(submenu.isConnected || screen.getByTestId('host-menu-move-to-menu')).toBeTruthy();
    await user.click(within(screen.getByTestId('host-menu-move-to-menu')).getByTestId('host-menu-move-to-group-1'));
    expect(onSelect).toHaveBeenCalledWith('group');
    expect(onAction).toHaveBeenCalledWith('group');
    expect(onAction).not.toHaveBeenCalledWith('move-to');
    expect(screen.queryByTestId('host-context-menu')).not.toBeInTheDocument();
  });

  it('navigates submenus with the keyboard', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.keyboard('{ArrowDown}{ArrowRight}');
    expect(screen.getByTestId('host-menu-move-to-top-level')).toHaveFocus();
    await user.keyboard('{ArrowLeft}');
    expect(screen.queryByTestId('host-menu-move-to-menu')).not.toBeInTheDocument();
    expect(screen.getByTestId('host-menu-move-to')).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(screen.getByTestId('host-menu-move-to-top-level')).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(screen.getByTestId('host-context-menu')).toBeInTheDocument();
    expect(screen.queryByTestId('host-menu-move-to-menu')).not.toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('host-context-menu')).not.toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('shows the info text on hover of the info icon without running the item', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    const item = screen.getByTestId('host-menu-copy-link');
    expect(item).toHaveAccessibleDescription(/someone who has it in a vault/);
    const text = screen.getByTestId('host-menu-copy-link-info-text');
    expect(text).not.toBeVisible();
    await user.hover(screen.getByTestId('host-menu-copy-link-info'));
    expect(text).toBeVisible();
    await user.click(screen.getByTestId('host-menu-copy-link-info'));
    expect(onSelect).not.toHaveBeenCalled();
    await user.unhover(screen.getByTestId('host-menu-copy-link-info'));
    expect(text).not.toBeVisible();
    // Keyboard focus shows it too.
    await user.keyboard('{Home}{ArrowDown}{ArrowDown}');
    expect(item).toHaveFocus();
    expect(text).toBeVisible();
  });

  it('closes on a press outside, but not on its trigger', async () => {
    const user = userEvent.setup();
    render(<Harness onSelect={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.click(screen.getByRole('button', { name: 'Open' }));
    // The trigger toggles it closed instead of closing and reopening.
    expect(screen.queryByTestId('host-context-menu')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.click(screen.getByRole('button', { name: 'Elsewhere' }));
    expect(screen.queryByTestId('host-context-menu')).not.toBeInTheDocument();
  });

  it('moves back inside the viewport', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 200,
      bottom: 300,
      width: 200,
      height: 300,
      toJSON: () => ({}),
    });
    vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(1000);
    vi.spyOn(window, 'innerHeight', 'get').mockReturnValue(700);
    render(<ContextMenu items={hostMenu(vi.fn())} at={{ x: 950, y: 600 }} onClose={vi.fn()} data-testid="host-context-menu" />);
    const level = screen.getByTestId('host-context-menu').parentElement as HTMLElement;
    expect(level.style.left).toBe('792px');
    expect(level.style.top).toBe('392px');
  });
});

describe('ContextMenu: keyboard entry into a hovered submenu', () => {
  it('moves focus into a submenu that hover already opened', async () => {
    const user = userEvent.setup();
    render(<Harness onSelect={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await user.hover(screen.getByTestId('host-menu-move-to'));
    expect(screen.getByTestId('host-menu-move-to-menu')).toBeInTheDocument();
    expect(screen.getByTestId('host-menu-move-to')).toHaveFocus();
    await user.keyboard('{ArrowRight}');
    expect(screen.getByTestId('host-menu-move-to-top-level')).toHaveFocus();
  });
});
