import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Toggle } from './Toggle';
import css from './Toggle.module.css?raw';
import styles from './Toggle.module.css';

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('Toggle', () => {
  it('renders an unchecked switch named by its label', () => {
    render(<Toggle label="Autoreconnect" data-testid="autoreconnect" />);

    const toggle = screen.getByRole('switch', { name: 'Autoreconnect' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(toggle).toHaveAttribute('type', 'button');
    expect(screen.getByTestId('autoreconnect')).toBe(toggle);
  });

  it('accepts aria-label when there is no visible label', () => {
    render(<Toggle aria-label="Copy on select" defaultChecked />);

    expect(screen.getByRole('switch', { name: 'Copy on select' })).toBeChecked();
  });

  it('flips on click and on a click on its label', async () => {
    const user = userEvent.setup();
    const onCheckedChange = vi.fn();
    render(<Toggle label="Record command history" onCheckedChange={onCheckedChange} />);

    const toggle = screen.getByRole('switch', { name: 'Record command history' });
    await user.click(toggle);
    expect(toggle).toBeChecked();
    expect(onCheckedChange).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByText('Record command history'));
    expect(toggle).not.toBeChecked();
    expect(onCheckedChange).toHaveBeenLastCalledWith(false);
    expect(onCheckedChange).toHaveBeenCalledTimes(2);
  });

  it('flips with Space and Enter after Tab', async () => {
    const user = userEvent.setup();
    render(<Toggle label="Blinking" />);

    await user.tab();
    const toggle = screen.getByRole('switch', { name: 'Blinking' });
    expect(toggle).toHaveFocus();

    await user.keyboard('[Space]');
    expect(toggle).toBeChecked();
    await user.keyboard('{Enter}');
    expect(toggle).not.toBeChecked();
  });

  it('follows the checked prop when controlled', async () => {
    const user = userEvent.setup();
    const onCheckedChange = vi.fn();

    function Controlled() {
      const [checked, setChecked] = useState(true);
      return <Toggle label="Controlled" checked={checked} onCheckedChange={setChecked} />;
    }

    render(
      <>
        <Toggle label="Fixed" checked={false} onCheckedChange={onCheckedChange} />
        <Controlled />
      </>,
    );

    await user.click(screen.getByRole('switch', { name: 'Fixed' }));
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(screen.getByRole('switch', { name: 'Fixed' })).not.toBeChecked();

    const controlled = screen.getByRole('switch', { name: 'Controlled' });
    expect(controlled).toBeChecked();
    await user.click(controlled);
    expect(controlled).not.toBeChecked();
  });

  it('does not flip when onClick prevents the default action', async () => {
    const user = userEvent.setup();
    const onCheckedChange = vi.fn();
    render(<Toggle label="Guarded" onClick={(event) => event.preventDefault()} onCheckedChange={onCheckedChange} />);

    await user.click(screen.getByRole('switch', { name: 'Guarded' }));
    expect(onCheckedChange).not.toHaveBeenCalled();
    expect(screen.getByRole('switch', { name: 'Guarded' })).not.toBeChecked();
  });

  it('ignores interaction and leaves the tab order when disabled', async () => {
    const user = userEvent.setup();
    const onCheckedChange = vi.fn();
    render(<Toggle label="Sync keys" disabled onCheckedChange={onCheckedChange} />);

    const toggle = screen.getByRole('switch', { name: 'Sync keys' });
    expect(toggle).toBeDisabled();
    expect(toggle.closest(`.${styles.root}`)).toHaveAttribute('data-disabled', 'true');

    await user.click(toggle);
    await user.click(screen.getByText('Sync keys'));
    await user.tab();
    expect(toggle).not.toHaveFocus();
    expect(onCheckedChange).not.toHaveBeenCalled();
    expect(toggle).not.toBeChecked();
  });

  it('links the description and supports the label at the start', () => {
    const ref = createRef<HTMLButtonElement>();
    render(
      <Toggle
        ref={ref}
        label="Automatic update check"
        description="Sends only the current version."
        labelPosition="start"
        className="row"
      />,
    );

    const toggle = screen.getByRole('switch', { name: 'Automatic update check' });
    expect(toggle).toHaveAccessibleDescription('Sends only the current version.');
    expect(ref.current).toBe(toggle);
    expect(toggle.closest(`.${styles.root}`)).toHaveClass(styles.labelStart, 'row');
  });
});

describe('Toggle styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('uses the accent track when on and the strong border track when off', () => {
    expect(css).toMatch(/\.switch\s*\{[^}]*background-color: var\(--border-strong\)/);
    expect(css).toMatch(/\.switch\[aria-checked='true'\]\s*\{\s*background-color: var\(--accent\);/);
    expect(css).toMatch(
      /:focus-visible\s*\{\s*outline: var\(--focus-ring-width\) solid var\(--focus-ring\);\s*outline-offset: var\(--focus-ring-offset\);/,
    );
    expect(css).toMatch(/\[data-disabled\]\s*\{[^}]*opacity: var\(--opacity-disabled\)/);
    expect(css).toContain('transition: background-color var(--duration-fast) var(--easing-standard)');
    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
    for (const match of css.matchAll(/^[^{}]*:(?:hover|active)[^{]*\{/gm)) {
      expect(match[0]).toContain(':not(:disabled)');
    }
  });
});
