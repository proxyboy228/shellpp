import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Monitor } from 'lucide-react';
import type { ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { HostCard, TAG_COLORS, type TagColor } from './HostCard';
import css from './HostCard.module.css?raw';
import styles from './HostCard.module.css';

function undefinedTokens(source: string): string[] {
  const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]).filter((name) => !defined.has(name));
}

function Grid({ children }: { children: ReactNode }) {
  return (
    <div role="listbox" aria-label="Hosts">
      {children}
    </div>
  );
}

const tileClass: Record<TagColor, string> = {
  graphite: styles.tagGraphite,
  jade: styles.tagJade,
  amber: styles.tagAmber,
  coral: styles.tagCoral,
  plum: styles.tagPlum,
  moss: styles.tagMoss,
  ocean: styles.tagOcean,
  rose: styles.tagRose,
};

function tileOf(card: HTMLElement): HTMLElement {
  const tile = card.querySelector<HTMLElement>(`.${styles.tile}`);
  if (tile === null) {
    throw new Error('No tile in the card');
  }
  return tile;
}

describe('HostCard', () => {
  it('renders a host card as an option with title and subtitle', () => {
    render(
      <Grid>
        <HostCard label="web-1" address="10.0.0.5" subtitle="SSH, root, Production" data-testid="card" />
      </Grid>,
    );

    const card = screen.getByRole('option', { name: /web-1/ });
    expect(card).toBe(screen.getByTestId('card'));
    expect(card).toHaveAttribute('aria-selected', 'false');
    expect(card).toHaveAttribute('tabindex', '0');
    expect(card).toHaveAttribute('data-variant', 'host');
    expect(card).toHaveClass(styles.card);
    expect(screen.getByText('web-1')).toHaveClass(styles.title);
    expect(screen.getByText('SSH, root, Production')).toHaveClass(styles.subtitle);
    expect(screen.queryByText('10.0.0.5')).not.toBeInTheDocument();
  });

  it('uses the address as the title when the label is empty', () => {
    render(
      <Grid>
        <HostCard label="  " address="db.example.internal" />
      </Grid>,
    );

    expect(screen.getByRole('option', { name: 'db.example.internal' })).toBeInTheDocument();
    expect(screen.getByText('db.example.internal')).toHaveAttribute('title', 'db.example.internal');
  });

  it('shows the neutral server icon by default and a given OS icon', () => {
    const { rerender } = render(
      <Grid>
        <HostCard address="10.0.0.5" data-testid="card" />
      </Grid>,
    );

    const tile = tileOf(screen.getByTestId('card'));
    expect(tile).toHaveAttribute('aria-hidden', 'true');
    expect(tile.querySelector('svg')).toHaveClass('lucide-server');

    rerender(
      <Grid>
        <HostCard address="10.0.0.5" icon={Monitor} data-testid="card" />
      </Grid>,
    );
    expect(tileOf(screen.getByTestId('card')).querySelector('svg')).toHaveClass('lucide-monitor');
  });

  it.each(TAG_COLORS)('paints the tile in the %s color', (color) => {
    render(
      <Grid>
        <HostCard address="10.0.0.5" color={color} data-testid="card" />
      </Grid>,
    );

    expect(tileOf(screen.getByTestId('card'))).toHaveClass(styles.tile, tileClass[color]);
    expect(screen.getByTestId('card')).toHaveAttribute('data-color', color);
  });

  it('defaults to the graphite tile', () => {
    render(
      <Grid>
        <HostCard address="10.0.0.5" data-testid="card" />
      </Grid>,
    );

    expect(tileOf(screen.getByTestId('card'))).toHaveClass(styles.tagGraphite);
  });

  it('renders the group variant with a folder icon and the host count', () => {
    const { rerender } = render(
      <Grid>
        <HostCard variant="group" label="Production" hostCount={3} data-testid="card" />
      </Grid>,
    );

    const card = screen.getByRole('option', { name: /Production/ });
    expect(card).toHaveAttribute('data-variant', 'group');
    expect(tileOf(card).querySelector('svg')).toHaveClass('lucide-folder');
    expect(screen.getByText('3 hosts')).toHaveClass(styles.subtitle);

    rerender(
      <Grid>
        <HostCard variant="group" label="Production" hostCount={1} />
      </Grid>,
    );
    expect(screen.getByText('1 host')).toBeInTheDocument();

    rerender(
      <Grid>
        <HostCard variant="group" label="Production" hostCount={5} formatHostCount={(count) => `Hosts: ${count}`} />
      </Grid>,
    );
    expect(screen.getByText('Hosts: 5')).toBeInTheDocument();
  });

  it('marks the selected card', () => {
    render(
      <Grid>
        <HostCard address="10.0.0.5" selected data-testid="card" />
      </Grid>,
    );

    expect(screen.getByRole('option', { selected: true })).toBe(screen.getByTestId('card'));
    expect(screen.getByTestId('card')).toHaveClass(styles.selected);
  });

  it('selects on click and Space, opens on double click and Enter', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onOpen = vi.fn();
    render(
      <Grid>
        <HostCard address="10.0.0.5" onSelect={onSelect} onOpen={onOpen} />
      </Grid>,
    );
    const card = screen.getByRole('option');

    await user.click(card);
    expect(onSelect).toHaveBeenCalledTimes(1);

    await user.dblClick(card);
    expect(onOpen).toHaveBeenCalledTimes(1);

    onSelect.mockClear();
    onOpen.mockClear();
    card.blur();
    await user.tab();
    expect(card).toHaveFocus();
    await user.keyboard(' ');
    expect(onSelect).toHaveBeenCalledTimes(1);
    await user.keyboard('{Enter}');
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('passes modifier keys to onSelect for multiple selection', () => {
    const onSelect = vi.fn();
    render(
      <Grid>
        <HostCard address="10.0.0.5" onSelect={onSelect} />
      </Grid>,
    );

    fireEvent.click(screen.getByRole('option'), { ctrlKey: true });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0]?.[0]).toMatchObject({ ctrlKey: true });
  });

  it('lets a handler cancel the default selection', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <Grid>
        <HostCard address="10.0.0.5" onSelect={onSelect} onClick={(event) => event.preventDefault()} />
      </Grid>,
    );

    await user.click(screen.getByRole('option'));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('ignores pointer and keyboard input when disabled', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onOpen = vi.fn();
    const onClick = vi.fn();
    render(
      <Grid>
        <HostCard address="10.0.0.5" disabled onSelect={onSelect} onOpen={onOpen} onClick={onClick} data-testid="card" />
      </Grid>,
    );
    const card = screen.getByTestId('card');

    expect(card).toHaveAttribute('aria-disabled', 'true');
    expect(card).toHaveAttribute('tabindex', '-1');
    expect(card).toHaveClass(styles.disabled);

    await user.click(card);
    await user.dblClick(card);
    fireEvent.keyDown(card, { key: 'Enter' });
    fireEvent.keyDown(card, { key: ' ' });
    expect(onSelect).not.toHaveBeenCalled();
    expect(onOpen).not.toHaveBeenCalled();
    expect(onClick).not.toHaveBeenCalled();
  });

  it('renders trailing content and leaves its keys alone', async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <Grid>
        <HostCard address="10.0.0.5" onOpen={onOpen} trailing={<button type="button">Resolve</button>} />
      </Grid>,
    );

    const button = screen.getByRole('button', { name: 'Resolve' });
    button.focus();
    await user.keyboard('{Enter}');
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('uses only design tokens and follows the state rules', () => {
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    expect(undefinedTokens(css)).toEqual([]);
    expect(css).toMatch(/height:\s*var\(--height-host-card\)/);
    expect(css).toMatch(/border-radius:\s*var\(--radius-tile\)/);
    expect(css).toMatch(/color:\s*var\(--text-on-tag\)/);
    expect(css).toMatch(/\.subtitle\s*\{[^}]*color:\s*var\(--text-secondary\)/);
    expect(css).toMatch(/text-overflow:\s*ellipsis/);
    expect(css).toMatch(/:not\(\.disabled\):hover\s*\{\s*background:\s*var\(--bg-raised-hover\)/);
    expect(css).toMatch(/:not\(\.disabled\):active\s*\{\s*background:\s*var\(--bg-pressed\)/);
    expect(css).toMatch(/\.selected\s*\{[^}]*border-color:\s*var\(--accent\)/);
    expect(css).toMatch(/\.disabled\s*\{[^}]*opacity:\s*var\(--opacity-disabled\)/);
    expect(css).toMatch(/:focus-visible\s*\{[^}]*outline:\s*var\(--focus-ring-width\) solid var\(--focus-ring\)/);
    expect(css).toMatch(/outline-offset:\s*var\(--focus-ring-offset\)/);
    for (const color of TAG_COLORS) {
      expect(css).toContain(`background: var(--tag-${color});`);
    }
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });
});
