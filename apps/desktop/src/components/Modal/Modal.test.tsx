import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Menu } from '../Menu/Menu';
import { Modal, type ModalProps } from './Modal';
import css from './Modal.module.css?raw';
import styles from './Modal.module.css';

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]).filter((name) => !defined.has(name));
}

function EditHostModal(props: Partial<ModalProps>) {
  return (
    <Modal
      open
      title="Edit host"
      onClose={() => undefined}
      footer={<button type="button">Save</button>}
      data-testid="modal"
      {...props}
    >
      <label>
        Address
        <input type="text" />
      </label>
    </Modal>
  );
}

describe('Modal', () => {
  it('renders nothing while closed', () => {
    render(<EditHostModal open={false} />);

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('renders a labelled modal dialog with title, content and footer', () => {
    render(<EditHostModal />);

    const dialog = screen.getByRole('dialog', { name: 'Edit host' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toBe(screen.getByTestId('modal'));
    expect(dialog).toHaveClass(styles.dialog, styles.medium);
    expect(screen.getByRole('heading', { level: 2, name: 'Edit host' })).toHaveClass(styles.title);
    expect(screen.getByRole('textbox', { name: 'Address' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(screen.getByTestId('modal-scrim')).toHaveClass(styles.scrim);
    expect(screen.getByTestId('modal-close')).toHaveAccessibleName('Close');
  });

  it('renders into document.body', () => {
    const { container } = render(<EditHostModal />);

    expect(container).toBeEmptyDOMElement();
    expect(screen.getByTestId('modal-scrim').parentElement).toBe(document.body);
  });

  it('applies the size variants', () => {
    const { rerender } = render(<EditHostModal size="small" />);
    expect(screen.getByRole('dialog')).toHaveClass(styles.small);

    rerender(<EditHostModal size="large" />);
    expect(screen.getByRole('dialog')).toHaveClass(styles.large);
  });

  it('closes on Escape and on the close button', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<EditHostModal onClose={onClose} closeLabel="Close dialog" />);

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole('button', { name: 'Close dialog' }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('closes on a scrim click only when asked to', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const { rerender } = render(<EditHostModal onClose={onClose} />);

    await user.click(screen.getByTestId('modal-scrim'));
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('textbox', { name: 'Address' })).toHaveFocus();

    await user.click(screen.getByRole('dialog'));
    expect(onClose).not.toHaveBeenCalled();

    rerender(<EditHostModal onClose={onClose} closeOnScrimClick />);
    await user.click(screen.getByTestId('modal-scrim'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('focuses the first focusable element of the content on open', () => {
    render(<EditHostModal />);

    expect(screen.getByRole('textbox', { name: 'Address' })).toHaveFocus();
  });

  it('focuses the close button when the content has nothing focusable', () => {
    render(
      <Modal open title="About" onClose={() => undefined}>
        <p>Shell++</p>
      </Modal>,
    );

    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();
  });

  it('focuses the element given by initialFocusRef', () => {
    function Harness() {
      const saveRef = useRef<HTMLButtonElement>(null);
      return (
        <EditHostModal
          initialFocusRef={saveRef}
          footer={
            <button type="button" ref={saveRef}>
              Save
            </button>
          }
        />
      );
    }
    render(<Harness />);

    expect(screen.getByRole('button', { name: 'Save' })).toHaveFocus();
  });

  it('keeps Tab and Shift+Tab inside the dialog', async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Outside</button>
        <EditHostModal />
      </>,
    );
    const close = screen.getByRole('button', { name: 'Close' });
    const input = screen.getByRole('textbox', { name: 'Address' });
    const save = screen.getByRole('button', { name: 'Save' });

    expect(input).toHaveFocus();
    await user.tab();
    expect(save).toHaveFocus();
    await user.tab();
    expect(close).toHaveFocus();
    await user.tab();
    expect(input).toHaveFocus();
    await user.tab({ shift: true });
    expect(close).toHaveFocus();
    await user.tab({ shift: true });
    expect(save).toHaveFocus();
  });

  it('returns focus to the trigger when it closes', async () => {
    const user = userEvent.setup();
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Edit
          </button>
          <Modal open={open} title="Edit host" onClose={() => setOpen(false)}>
            <input type="text" aria-label="Address" />
          </Modal>
        </>
      );
    }
    render(<Harness />);

    const trigger = screen.getByRole('button', { name: 'Edit' });
    await user.click(trigger);
    expect(screen.getByRole('textbox', { name: 'Address' })).toHaveFocus();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it('lets Escape close a menu inside it without closing the dialog', async () => {
    const user = userEvent.setup();
    const onModalClose = vi.fn();
    const onMenuClose = vi.fn();
    render(
      <Modal open title="Edit host" onClose={onModalClose}>
        <Menu items={[{ id: 'copy', label: 'Copy' }]} aria-label="Actions" onClose={onMenuClose} />
      </Modal>,
    );

    expect(screen.getByRole('menuitem', { name: 'Copy' })).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(onMenuClose).toHaveBeenCalledTimes(1);
    expect(onModalClose).not.toHaveBeenCalled();
  });

  it('uses only design tokens and follows the style rules', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    expect(undefinedTokens(css)).toEqual([]);
    expect(css).toMatch(/\.scrim\s*\{[^}]*background:\s*var\(--scrim\)/);
    expect(css).toMatch(/\.dialog\s*\{[^}]*border-radius:\s*var\(--radius-overlay\)/);
    expect(css).toMatch(/\.dialog\s*\{[^}]*background:\s*var\(--bg-overlay\)/);
    expect(css).toMatch(/\.dialog\s*\{[^}]*box-shadow:\s*var\(--shadow-modal\)/);
    expect(css).toMatch(/\.title\s*\{[^}]*font-size:\s*var\(--font-size-title\)/);
    expect(css).toMatch(/\.close:hover\s*\{[^}]*background:\s*var\(--bg-overlay-hover\)/);
    expect(css).toMatch(/\.close:active\s*\{[^}]*background:\s*var\(--bg-pressed\)/);
    expect(css).toMatch(/:focus-visible\s*\{[^}]*outline:\s*var\(--focus-ring-width\) solid var\(--focus-ring\)/);
    expect(css).toMatch(/animation:\s*dialog-in var\(--duration-normal\) var\(--easing-standard\)/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.scrim,\s*\.dialog\s*\{\s*animation:\s*none/);
  });
});
