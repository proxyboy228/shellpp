import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Plus } from 'lucide-react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Button } from './Button';
import type { ButtonVariant } from './Button';
import css from './Button.module.css?raw';
import styles from './Button.module.css';

const VARIANTS: readonly ButtonVariant[] = ['primary', 'secondary', 'danger', 'text'];

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('Button', () => {
  it('renders a non-submitting secondary button by default', () => {
    render(<Button>Save</Button>);

    const button = screen.getByRole('button', { name: 'Save' });
    expect(button).toHaveAttribute('type', 'button');
    expect(button).toHaveAttribute('data-variant', 'secondary');
    expect(button).toHaveClass(styles.button, styles.secondary);
    expect(button).toBeEnabled();
    expect(button).not.toHaveAttribute('aria-busy');
  });

  it.each(VARIANTS)('applies the %s variant', (variant) => {
    render(<Button variant={variant}>Action</Button>);

    const button = screen.getByRole('button', { name: 'Action' });
    expect(button).toHaveAttribute('data-variant', variant);
    expect(button).toHaveClass(styles[variant]);
  });

  it('forwards data-testid, className, ref and other attributes', () => {
    const ref = createRef<HTMLButtonElement>();
    render(
      <Button ref={ref} data-testid="connect" className="extra" aria-describedby="hint" title="Connect now">
        Connect
      </Button>,
    );

    const button = screen.getByTestId('connect');
    expect(button).toHaveClass('extra', styles.button);
    expect(button).toHaveAttribute('aria-describedby', 'hint');
    expect(button).toHaveAttribute('title', 'Connect now');
    expect(ref.current).toBe(button);
  });

  it('renders a leading icon hidden from assistive technology', () => {
    render(<Button icon={<Plus data-testid="icon" />}>New host</Button>);

    const icon = screen.getByTestId('icon');
    expect(icon.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'New host' })).toContainElement(icon);
  });

  it('calls onClick on click, Enter and Space', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Run</Button>);

    await user.tab();
    expect(screen.getByRole('button', { name: 'Run' })).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onClick).toHaveBeenCalledTimes(1);
    await user.keyboard('[Space]');
    expect(onClick).toHaveBeenCalledTimes(2);

    await user.click(screen.getByRole('button', { name: 'Run' }));
    expect(onClick).toHaveBeenCalledTimes(3);
  });

  it('ignores clicks and leaves the tab order when disabled', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        Delete
      </Button>,
    );

    const button = screen.getByRole('button', { name: 'Delete' });
    expect(button).toBeDisabled();

    await user.click(button);
    await user.tab();
    expect(onClick).not.toHaveBeenCalled();
    expect(button).not.toHaveFocus();
  });

  it('shows a spinner, stays focusable and ignores activation while loading', async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
    render(
      <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
        <Button type="submit" loading onClick={onClick} icon={<Plus data-testid="icon" />}>
          Continue
        </Button>
      </form>,
    );

    const button = screen.getByRole('button', { name: 'Continue' });
    expect(button).toHaveAttribute('aria-busy', 'true');
    expect(button).toHaveAttribute('aria-disabled', 'true');
    expect(button).not.toBeDisabled();
    expect(screen.queryByTestId('icon')).not.toBeInTheDocument();
    expect(button.querySelector(`.${styles.spinner}`)).not.toBeNull();

    await user.tab();
    expect(button).toHaveFocus();
    await user.keyboard('{Enter}');
    await user.click(button);
    expect(onClick).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('submits a form when type is submit', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn((event: SubmitEvent) => event.preventDefault());
    render(
      <form onSubmit={(event) => onSubmit(event.nativeEvent as SubmitEvent)}>
        <Button type="submit" variant="primary">
          Continue
        </Button>
      </form>,
    );

    await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});

describe('Button styles', () => {
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
    // Hover and press never apply to a disabled button.
    for (const match of css.matchAll(/^[^{}]*:(?:hover|active)[^{]*\{/gm)) {
      expect(match[0]).toContain(':not(:disabled');
    }
  });
});
