import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Chip } from './Chip';
import css from './Chip.module.css?raw';
import styles from './Chip.module.css';

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]).filter((name) => !defined.has(name));
}

describe('Chip', () => {
  it('renders a neutral chip by default', () => {
    render(<Chip label="production" data-testid="chip" />);

    const chip = screen.getByTestId('chip');
    expect(chip).toHaveTextContent('production');
    expect(chip).toHaveClass(styles.chip);
    expect(chip).not.toHaveClass(styles.active);
    expect(chip).not.toHaveAttribute('data-active');
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('renders the active filter style', () => {
    render(<Chip label="production" active data-testid="chip" />);

    expect(screen.getByTestId('chip')).toHaveClass(styles.active);
    expect(screen.getByTestId('chip')).toHaveAttribute('data-active', 'true');
  });

  it('forwards the class name and other attributes', () => {
    render(<Chip label="db" className="extra" title="Tag db" data-testid="chip" />);

    expect(screen.getByTestId('chip')).toHaveClass(styles.chip, 'extra');
    expect(screen.getByTestId('chip')).toHaveAttribute('title', 'Tag db');
  });

  it('removes with a labelled button, by click and by keyboard', async () => {
    const user = userEvent.setup();
    const onRemove = vi.fn();
    render(<Chip label="production" onRemove={onRemove} data-testid="chip" />);

    const remove = screen.getByRole('button', { name: 'Remove production' });
    expect(remove).toHaveAttribute('data-testid', 'chip-remove');

    await user.tab();
    expect(remove).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.keyboard(' ');
    expect(onRemove).toHaveBeenCalledTimes(2);

    await user.click(remove);
    expect(onRemove).toHaveBeenCalledTimes(3);
  });

  it('accepts a localized remove label', () => {
    render(<Chip label="prod" onRemove={() => undefined} removeLabel="Delete tag prod" />);

    expect(screen.getByRole('button', { name: 'Delete tag prod' })).toBeInTheDocument();
  });

  it('works as a filter toggle when onClick is given', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    const { rerender } = render(<Chip label="staging" onClick={onClick} />);

    const toggle = screen.getByRole('button', { name: 'staging' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await user.click(toggle);
    expect(onClick).toHaveBeenCalledTimes(1);

    rerender(<Chip label="staging" onClick={onClick} active />);
    expect(screen.getByRole('button', { name: 'staging' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('ignores clicks and keys when disabled', async () => {
    const user = userEvent.setup();
    const onRemove = vi.fn();
    const onClick = vi.fn();
    render(<Chip label="prod" disabled onClick={onClick} onRemove={onRemove} data-testid="chip" />);

    const chip = screen.getByTestId('chip');
    expect(chip).toHaveClass(styles.disabled);
    expect(chip).toHaveAttribute('aria-disabled', 'true');

    const remove = screen.getByRole('button', { name: 'Remove prod' });
    const toggle = screen.getByRole('button', { name: 'prod' });
    expect(remove).toBeDisabled();
    expect(toggle).toBeDisabled();

    await user.click(remove);
    await user.click(toggle);
    await user.tab();
    expect(remove).not.toHaveFocus();
    expect(onRemove).not.toHaveBeenCalled();
    expect(onClick).not.toHaveBeenCalled();
  });

  it('uses only design tokens and follows the state rules', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    expect(undefinedTokens(css)).toEqual([]);
    expect(css).toMatch(/\.active\s*\{[^}]*background:\s*var\(--accent-subtle\)[^}]*color:\s*var\(--text-primary\)/);
    expect(css).toMatch(/border:\s*1px solid var\(--border-strong\)/);
    expect(css).toMatch(/border-radius:\s*var\(--radius-control\)/);
    expect(css).toMatch(/\.disabled\s*\{[^}]*opacity:\s*var\(--opacity-disabled\)/);
    expect(css).toMatch(/:focus-visible[^{]*\{[^}]*outline:\s*var\(--focus-ring-width\) solid var\(--focus-ring\)/);
    expect(css).toMatch(/outline-offset:\s*var\(--focus-ring-offset\)/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
