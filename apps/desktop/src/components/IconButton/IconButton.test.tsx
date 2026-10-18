import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { LayoutGrid, Trash2 } from 'lucide-react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { IconButton } from './IconButton';
import type { IconButtonVariant } from './IconButton';
import css from './IconButton.module.css?raw';
import styles from './IconButton.module.css';

const VARIANTS: readonly IconButtonVariant[] = ['ghost', 'secondary', 'danger'];

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('IconButton', () => {
  it('takes its accessible name from aria-label and hides the icon', () => {
    render(<IconButton aria-label="Delete host" icon={<Trash2 data-testid="icon" />} />);

    const button = screen.getByRole('button', { name: 'Delete host' });
    expect(button).toHaveAttribute('type', 'button');
    expect(button).toHaveAttribute('data-variant', 'ghost');
    expect(button).toHaveClass(styles.iconButton, styles.ghost, styles.md);
    expect(button).not.toHaveAttribute('aria-pressed');
    expect(screen.getByTestId('icon').closest('[aria-hidden="true"]')).not.toBeNull();
    expect(button).toHaveTextContent('');
  });

  it('requires aria-label at the type level', () => {
    // @ts-expect-error: an icon-only button must have an accessible name.
    const element = <IconButton icon={<Trash2 />} />;
    expect(element).toBeTruthy();
  });

  it.each(VARIANTS)('applies the %s variant', (variant) => {
    render(<IconButton aria-label="Action" variant={variant} icon={<Trash2 />} />);

    const button = screen.getByRole('button', { name: 'Action' });
    expect(button).toHaveAttribute('data-variant', variant);
    expect(button).toHaveClass(styles[variant]);
  });

  it('applies the small size', () => {
    render(<IconButton aria-label="Show" size="sm" icon={<Trash2 />} />);

    expect(screen.getByRole('button', { name: 'Show' })).toHaveClass(styles.sm);
  });

  it('forwards data-testid, className, ref and other attributes', () => {
    const ref = createRef<HTMLButtonElement>();
    render(
      <IconButton
        ref={ref}
        aria-label="Settings"
        icon={<Trash2 />}
        data-testid="settings"
        className="extra"
        title="Settings"
      />,
    );

    const button = screen.getByTestId('settings');
    expect(button).toHaveClass('extra');
    expect(button).toHaveAttribute('title', 'Settings');
    expect(ref.current).toBe(button);
  });

  it('works as a toggle button with aria-pressed', () => {
    const { rerender } = render(<IconButton aria-label="Tiles" pressed={false} icon={<LayoutGrid />} />);
    expect(screen.getByRole('button', { name: 'Tiles', pressed: false })).toBeInTheDocument();

    rerender(<IconButton aria-label="Tiles" pressed icon={<LayoutGrid />} />);
    expect(screen.getByRole('button', { name: 'Tiles', pressed: true })).toBeInTheDocument();
  });

  it('activates with click, Enter and Space', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<IconButton aria-label="Refresh" icon={<Trash2 />} onClick={onClick} />);

    await user.tab();
    expect(screen.getByRole('button', { name: 'Refresh' })).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.keyboard('[Space]');
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(onClick).toHaveBeenCalledTimes(3);
  });

  it('ignores activation and leaves the tab order when disabled', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<IconButton aria-label="Refresh" icon={<Trash2 />} onClick={onClick} disabled />);

    const button = screen.getByRole('button', { name: 'Refresh' });
    expect(button).toBeDisabled();
    await user.click(button);
    await user.tab();
    expect(button).not.toHaveFocus();
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe('IconButton styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('defines the focus ring, the disabled state and reduced motion', () => {
    expect(css).toMatch(
      /:focus-visible\s*\{\s*outline: var\(--focus-ring-width\) solid var\(--focus-ring\);\s*outline-offset: var\(--focus-ring-offset\);/,
    );
    expect(css).toMatch(/:disabled\s*\{[^}]*opacity: var\(--opacity-disabled\)/);
    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
    for (const match of css.matchAll(/^[^{}]*:(?:hover|active)[^{]*\{/gm)) {
      expect(match[0]).toContain(':not(:disabled)');
    }
  });
});
