// DetailsPanel (UI 1.3, W4/W5 contract): header, menu, collapse, resize with
// the `view.details_panel_width` setting.

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetApp, setupBackend } from '../../test/app';
import { settingsStore } from '../store';
import { DetailsPanel, clampPanelWidth } from './DetailsPanel';

afterEach(() => {
  resetApp();
});

describe('DetailsPanel', () => {
  it('renders the header, body and footer with the sub test ids', async () => {
    const user = userEvent.setup();
    const onCollapse = vi.fn();
    render(
      <DetailsPanel title="Host details" onCollapse={onCollapse} footer={<p>Footer</p>} data-testid="host-panel">
        <input aria-label="Address" />
      </DetailsPanel>,
    );
    expect(screen.getByTestId('host-panel')).toHaveAccessibleName('Host details');
    expect(screen.getByTestId('host-panel-title')).toHaveTextContent('Host details');
    expect(screen.getByTestId('host-panel-body')).toContainElement(screen.getByLabelText('Address'));
    expect(screen.getByText('Footer')).toBeInTheDocument();
    expect(screen.queryByTestId('host-panel-menu-button')).not.toBeInTheDocument();
    expect(screen.getByTestId('host-panel')).toHaveStyle({ width: '380px' });

    await user.click(screen.getByTestId('host-panel-collapse'));
    expect(onCollapse).toHaveBeenCalledTimes(1);
    await user.click(screen.getByLabelText('Address'));
    await user.keyboard('{Escape}');
    expect(onCollapse).toHaveBeenCalledTimes(2);
  });

  it('opens the "..." menu with submenus', async () => {
    const user = userEvent.setup();
    const remove = vi.fn();
    const moveTop = vi.fn();
    render(
      <DetailsPanel
        title="Group details"
        onCollapse={vi.fn()}
        menuItems={[
          {
            id: 'move-to',
            label: 'Move to',
            'data-testid': 'group-panel-menu-move-to',
            children: [{ id: 'top', label: 'Top level', 'data-testid': 'group-panel-menu-move-to-top-level', onSelect: moveTop }],
          },
          { id: 'remove', label: 'Remove', danger: true, 'data-testid': 'group-panel-menu-remove', onSelect: remove },
        ]}
        data-testid="group-panel"
      >
        <p>Body</p>
      </DetailsPanel>,
    );
    await user.click(screen.getByTestId('group-panel-menu-button'));
    expect(screen.getByTestId('group-panel-menu')).toBeInTheDocument();
    // Escape closes the menu only, not the panel.
    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('group-panel-menu')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('group-panel-menu-button'));
    await user.click(screen.getByTestId('group-panel-menu-move-to'));
    await user.click(screen.getByTestId('group-panel-menu-move-to-top-level'));
    expect(moveTop).toHaveBeenCalled();
    await user.click(screen.getByTestId('group-panel-menu-button'));
    await user.click(screen.getByTestId('group-panel-menu-remove'));
    expect(remove).toHaveBeenCalled();
  });

  it('takes its width from the setting and stores a new width after dragging', async () => {
    const backend = setupBackend({ initialState: 'unlocked', settings: { 'view.details_panel_width': 400 } });
    act(() => {
      settingsStore.set({ 'view.details_panel_width': 400 });
    });
    render(
      <DetailsPanel title="Host details" onCollapse={vi.fn()} data-testid="host-panel">
        <p>Body</p>
      </DetailsPanel>,
    );
    const panel = screen.getByTestId('host-panel');
    expect(panel).toHaveStyle({ width: '400px' });
    const handle = screen.getByTestId('host-panel-resize-handle');
    fireEvent.pointerDown(handle, { button: 0, clientX: 600 });
    fireEvent.pointerMove(window, { clientX: 550 });
    expect(panel).toHaveStyle({ width: '450px' });
    // Clamped to 560 px.
    fireEvent.pointerMove(window, { clientX: 100 });
    expect(panel).toHaveStyle({ width: '560px' });
    fireEvent.pointerMove(window, { clientX: 580 });
    fireEvent.pointerUp(window, { clientX: 580 });
    await waitFor(() => expect(settingsStore.get()['view.details_panel_width']).toBe(420));
    expect((await backend.invoke('settings_get', undefined))['view.details_panel_width']).toBe(420);
    expect(panel).toHaveStyle({ width: '420px' });
  });

  it('resizes with the arrow keys on the handle', async () => {
    setupBackend({ initialState: 'unlocked' });
    const user = userEvent.setup();
    render(
      <DetailsPanel title="Host details" onCollapse={vi.fn()} data-testid="host-panel">
        <p>Body</p>
      </DetailsPanel>,
    );
    const handle = screen.getByTestId('host-panel-resize-handle');
    expect(handle).toHaveAttribute('aria-valuenow', '380');
    handle.focus();
    await user.keyboard('{ArrowLeft}');
    await waitFor(() => expect(settingsStore.get()['view.details_panel_width']).toBe(396));
    await user.keyboard('{Home}');
    await waitFor(() => expect(settingsStore.get()['view.details_panel_width']).toBe(320));
  });

  it('clamps widths to 320-560', () => {
    expect(clampPanelWidth(100)).toBe(320);
    expect(clampPanelWidth(900)).toBe(560);
    expect(clampPanelWidth(Number.NaN)).toBe(380);
    expect(clampPanelWidth(401.6)).toBe(402);
  });
});
