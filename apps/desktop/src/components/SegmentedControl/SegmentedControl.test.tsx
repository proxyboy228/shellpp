import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { LayoutGrid, List } from 'lucide-react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { SegmentedControl } from './SegmentedControl';
import type { SegmentedControlOption } from './SegmentedControl';
import css from './SegmentedControl.module.css?raw';
import styles from './SegmentedControl.module.css';

type Forwarding = 'local' | 'remote' | 'dynamic';

const OPTIONS: readonly SegmentedControlOption<Forwarding>[] = [
  { value: 'local', label: 'Local' },
  { value: 'remote', label: 'Remote' },
  { value: 'dynamic', label: 'Dynamic' },
];

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('SegmentedControl', () => {
  it('renders a named radio group with one checked radio', () => {
    render(<SegmentedControl options={OPTIONS} defaultValue="remote" aria-label="Forwarding type" data-testid="type" />);

    const group = screen.getByRole('radiogroup', { name: 'Forwarding type' });
    expect(screen.getByTestId('type')).toBe(group);

    const radios = screen.getAllByRole('radio');
    expect(radios.map((radio) => radio.textContent)).toEqual(['Local', 'Remote', 'Dynamic']);
    expect(screen.getByRole('radio', { name: 'Remote' })).toBeChecked();
    expect(screen.getByRole('radio', { name: 'Local' })).not.toBeChecked();
    expect(screen.getByTestId('type-dynamic')).toBe(screen.getByRole('radio', { name: 'Dynamic' }));
  });

  it('keeps only the selected segment in the tab order', async () => {
    const user = userEvent.setup();
    render(<SegmentedControl options={OPTIONS} defaultValue="dynamic" aria-label="Type" />);

    expect(screen.getByRole('radio', { name: 'Dynamic' })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('radio', { name: 'Local' })).toHaveAttribute('tabindex', '-1');
    expect(screen.getByRole('radio', { name: 'Remote' })).toHaveAttribute('tabindex', '-1');

    await user.tab();
    expect(screen.getByRole('radio', { name: 'Dynamic' })).toHaveFocus();
  });

  it('makes the first enabled segment the tab stop when nothing is selected', () => {
    render(
      <SegmentedControl
        options={[{ value: 'a', label: 'A', disabled: true }, { value: 'b', label: 'B' }]}
        aria-label="Letters"
      />,
    );

    expect(screen.getByRole('radio', { name: 'B' })).toHaveAttribute('tabindex', '0');
    expect(screen.getAllByRole('radio').every((radio) => radio.getAttribute('aria-checked') === 'false')).toBe(true);
  });

  it('selects on click and reports the value', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    render(<SegmentedControl options={OPTIONS} defaultValue="local" onValueChange={onValueChange} aria-label="Type" />);

    await user.click(screen.getByRole('radio', { name: 'Dynamic' }));
    expect(screen.getByRole('radio', { name: 'Dynamic' })).toBeChecked();
    expect(onValueChange).toHaveBeenCalledWith('dynamic');

    await user.click(screen.getByRole('radio', { name: 'Dynamic' }));
    expect(onValueChange).toHaveBeenCalledTimes(1);
  });

  it('moves the selection with arrow keys, wraps, and jumps with Home and End', async () => {
    const user = userEvent.setup();
    render(<SegmentedControl options={OPTIONS} defaultValue="local" aria-label="Type" />);
    const radio = (name: string) => screen.getByRole('radio', { name });

    await user.tab();
    await user.keyboard('{ArrowRight}');
    expect(radio('Remote')).toHaveFocus();
    expect(radio('Remote')).toBeChecked();

    await user.keyboard('{ArrowDown}');
    expect(radio('Dynamic')).toBeChecked();
    await user.keyboard('{ArrowRight}');
    expect(radio('Local')).toHaveFocus();
    expect(radio('Local')).toBeChecked();

    await user.keyboard('{ArrowLeft}');
    expect(radio('Dynamic')).toBeChecked();
    await user.keyboard('{ArrowUp}');
    expect(radio('Remote')).toBeChecked();

    await user.keyboard('{Home}');
    expect(radio('Local')).toHaveFocus();
    expect(radio('Local')).toBeChecked();
    await user.keyboard('{End}');
    expect(radio('Dynamic')).toHaveFocus();
    expect(radio('Dynamic')).toBeChecked();
  });

  it('selects the focused segment with Space', async () => {
    const user = userEvent.setup();
    render(<SegmentedControl options={OPTIONS} aria-label="Type" />);

    await user.tab();
    expect(screen.getByRole('radio', { name: 'Local' })).toHaveFocus();
    expect(screen.getByRole('radio', { name: 'Local' })).not.toBeChecked();
    await user.keyboard('[Space]');
    expect(screen.getByRole('radio', { name: 'Local' })).toBeChecked();
  });

  it('skips disabled segments', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    render(
      <SegmentedControl
        options={[OPTIONS[0], { ...OPTIONS[1], disabled: true }, OPTIONS[2]]}
        defaultValue="local"
        onValueChange={onValueChange}
        aria-label="Type"
      />,
    );

    expect(screen.getByRole('radio', { name: 'Remote' })).toBeDisabled();
    await user.click(screen.getByRole('radio', { name: 'Remote' }));
    expect(onValueChange).not.toHaveBeenCalled();

    await user.tab();
    await user.keyboard('{ArrowRight}');
    expect(screen.getByRole('radio', { name: 'Dynamic' })).toBeChecked();
    expect(onValueChange).toHaveBeenCalledWith('dynamic');
  });

  it('follows the value prop when controlled', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();

    function Controlled() {
      const [value, setValue] = useState<Forwarding>('local');
      return <SegmentedControl options={OPTIONS} value={value} onValueChange={setValue} aria-label="Controlled" />;
    }

    const { rerender } = render(
      <SegmentedControl options={OPTIONS} value="local" onValueChange={onValueChange} aria-label="Fixed" />,
    );
    await user.click(screen.getByRole('radio', { name: 'Remote' }));
    expect(onValueChange).toHaveBeenCalledWith('remote');
    expect(screen.getByRole('radio', { name: 'Local' })).toBeChecked();

    rerender(<Controlled />);
    await user.click(screen.getByRole('radio', { name: 'Remote' }));
    expect(screen.getByRole('radio', { name: 'Remote' })).toBeChecked();
  });

  it('disables the whole group', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    render(
      <SegmentedControl options={OPTIONS} defaultValue="local" disabled onValueChange={onValueChange} aria-label="Type" />,
    );

    const group = screen.getByRole('radiogroup', { name: 'Type' });
    expect(group).toHaveAttribute('aria-disabled', 'true');
    expect(group).toHaveAttribute('data-disabled', 'true');
    for (const radio of screen.getAllByRole('radio')) {
      expect(radio).toBeDisabled();
    }

    await user.click(screen.getByRole('radio', { name: 'Remote' }));
    await user.tab();
    expect(onValueChange).not.toHaveBeenCalled();
    expect(document.body).toHaveFocus();
  });

  it('supports icon-only segments, custom test ids and full width', () => {
    render(
      <SegmentedControl
        options={[
          { value: 'tiles', icon: <LayoutGrid data-testid="tiles-icon" />, 'aria-label': 'Tiles' },
          { value: 'list', icon: <List />, 'aria-label': 'List', 'data-testid': 'list-view' },
        ]}
        defaultValue="tiles"
        aria-labelledby="view-label"
        fullWidth
        data-testid="view"
      />,
    );

    expect(screen.getByRole('radio', { name: 'Tiles' })).toBeChecked();
    expect(screen.getByTestId('tiles-icon').closest('[aria-hidden="true"]')).not.toBeNull();
    expect(screen.getByTestId('list-view')).toBe(screen.getByRole('radio', { name: 'List' }));
    expect(screen.getByTestId('view')).toHaveAttribute('aria-labelledby', 'view-label');
    expect(screen.getByTestId('view')).toHaveClass(styles.fullWidth);
  });
});

describe('SegmentedControl styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('defines the selection, focus ring, disabled state and reduced motion', () => {
    expect(css).toContain('background-color: var(--accent-subtle)');
    expect(css).toMatch(
      /:focus-visible\s*\{[^}]*outline: var\(--focus-ring-width\) solid var\(--focus-ring\);\s*outline-offset: var\(--focus-ring-offset\);/,
    );
    expect(css).toMatch(/\[data-disabled\]\s*\{[^}]*opacity: var\(--opacity-disabled\)/);
    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
    for (const match of css.matchAll(/^[^{}]*:(?:hover|active)[^{]*\{/gm)) {
      expect(match[0]).toContain(':not(:disabled)');
    }
  });
});
