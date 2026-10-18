import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { STATUS_DOT_LABELS, StatusDot, type StatusDotStatus } from './StatusDot';
import css from './StatusDot.module.css?raw';
import styles from './StatusDot.module.css';

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]).filter((name) => !defined.has(name));
}

const STATUSES: readonly StatusDotStatus[] = ['connected', 'connecting', 'error', 'inactive'];

describe('StatusDot', () => {
  it.each(STATUSES)('renders the %s state with an accessible label', (status) => {
    render(<StatusDot status={status} data-testid="dot" />);

    const dot = screen.getByRole('img', { name: STATUS_DOT_LABELS[status] });
    expect(dot).toBe(screen.getByTestId('dot'));
    expect(dot).toHaveClass(styles.dot, styles[status]);
    expect(dot).toHaveAttribute('data-status', status);
  });

  it('accepts a localized label', () => {
    render(<StatusDot status="connecting" label="Connecting to web-1" />);

    expect(screen.getByRole('img', { name: 'Connecting to web-1' })).toBeInTheDocument();
  });

  it('can be hidden from assistive technology when text repeats it', () => {
    render(<StatusDot status="error" decorative data-testid="dot" />);

    const dot = screen.getByTestId('dot');
    expect(dot).toHaveAttribute('aria-hidden', 'true');
    expect(dot).not.toHaveAttribute('role');
    expect(dot).not.toHaveAttribute('aria-label');
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });

  it('forwards the class name', () => {
    render(<StatusDot status="inactive" className="extra" data-testid="dot" />);

    expect(screen.getByTestId('dot')).toHaveClass(styles.dot, 'extra');
  });

  it('maps the states to the semantic tokens and uses the 8 px size token', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    expect(undefinedTokens(css)).toEqual([]);
    expect(css).toMatch(/\.connected\s*\{\s*background:\s*var\(--success\);/);
    expect(css).toMatch(/\.connecting\s*\{\s*background:\s*var\(--warning\);/);
    expect(css).toMatch(/\.error\s*\{\s*background:\s*var\(--danger\);/);
    expect(css).toMatch(/\.inactive\s*\{\s*background:\s*var\(--text-muted\);/);
    expect(css).toMatch(/width:\s*var\(--size-status-dot\)/);
    expect(css).toMatch(/height:\s*var\(--size-status-dot\)/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
