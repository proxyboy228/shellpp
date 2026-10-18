// Smoke test of the showcase page: it renders every section and component,
// shows a swatch for every color token and switches the interface theme.
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import tokensCss from '../theme/tokens.css?raw';
import { BUILTIN_TERMINAL_THEMES } from '../theme/builtin-terminal-themes';
import { parseThemeTokens } from '../theme/tokens';
import { App, SHOWCASE_TITLE } from './App';
import { COMPONENT_SPECIMENS } from './Components';

const tokens = parseThemeTokens(tokensCss);

/** Every color and shadow token: the theme blocks plus the shared tile colors. */
const colorTokenNames = [
  ...tokens.dark.keys(),
  ...[...tokens.shared.keys()].filter((name) => name.startsWith('--tag-') || name === '--text-on-tag'),
];

function swatchValue(name: string): HTMLElement {
  return screen.getByTestId(`swatch${name.slice(1)}-value`);
}

function themeRadio(name: 'Auto' | 'Light' | 'Dark'): HTMLElement {
  const group = screen.getByRole('radiogroup', { name: 'Theme' });
  return within(group).getByRole('radio', { name });
}

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
});

describe('Showcase', () => {
  it('renders the title, the sections and every base component', () => {
    render(<App />);

    expect(screen.getByRole('heading', { level: 1, name: SHOWCASE_TITLE })).toBeInTheDocument();
    expect(SHOWCASE_TITLE).toBe('Shell++ design system');
    expect(document.title).toBe(SHOWCASE_TITLE);

    for (const title of ['Colors', 'Typography', 'Spacing and shape', 'Terminal themes', 'Components']) {
      expect(screen.getByRole('heading', { level: 2, name: title })).toBeInTheDocument();
    }
    for (const { title } of COMPONENT_SPECIMENS) {
      expect(screen.getByRole('heading', { level: 3, name: title })).toBeInTheDocument();
    }

    const navigation = screen.getByRole('navigation', { name: 'Sections' });
    expect(within(navigation).getByRole('link', { name: 'Components' })).toHaveAttribute('href', '#components');
    expect(within(navigation).getByRole('link', { name: 'HostCard' })).toHaveAttribute('href', '#host-card');
  });

  it('shows a swatch for every color token', () => {
    render(<App />);

    expect(colorTokenNames.length).toBeGreaterThan(40);
    for (const name of colorTokenNames) {
      expect(screen.getByTestId(`swatch${name.slice(1)}`)).toHaveTextContent(name);
    }
  });

  it('previews both built-in terminal themes with 16 ANSI colors and their roles', () => {
    render(<App />);

    expect(BUILTIN_TERMINAL_THEMES).toHaveLength(2);
    for (const theme of BUILTIN_TERMINAL_THEMES) {
      const preview = screen.getByRole('figure', { name: new RegExp(theme.name) });
      const ansi = within(preview).getByRole('list', { name: 'ANSI colors' });
      expect(within(ansi).getAllByRole('listitem')).toHaveLength(16);
      expect(ansi).toHaveTextContent(theme.ansi.brightWhite);

      const roles = within(preview).getByRole('list', { name: 'Roles' });
      for (const role of ['background', 'foreground', 'cursor', 'selection']) {
        expect(within(roles).getByText(role)).toBeInTheDocument();
      }
      expect(roles).toHaveTextContent(theme.selection);
    }
  });

  it('switches the theme between Auto, Light and Dark', async () => {
    const user = userEvent.setup();
    render(<App />);
    const root = document.documentElement;

    // Auto: no attribute; the inert matchMedia of the test setup reports a dark system theme.
    expect(themeRadio('Auto')).toBeChecked();
    expect(root).not.toHaveAttribute('data-theme');
    expect(screen.getByTestId('showcase-effective-theme')).toHaveTextContent('System: dark');
    expect(swatchValue('--bg-base')).toHaveTextContent(tokens.dark.get('--bg-base') ?? '');

    await user.click(themeRadio('Light'));
    expect(themeRadio('Light')).toBeChecked();
    expect(root).toHaveAttribute('data-theme', 'light');
    expect(swatchValue('--bg-base')).toHaveTextContent(tokens.light.get('--bg-base') ?? '');
    expect(swatchValue('--accent')).toHaveTextContent(tokens.light.get('--accent') ?? '');

    await user.click(themeRadio('Dark'));
    expect(root).toHaveAttribute('data-theme', 'dark');
    expect(swatchValue('--bg-base')).toHaveTextContent(tokens.dark.get('--bg-base') ?? '');

    await user.click(themeRadio('Auto'));
    expect(root).not.toHaveAttribute('data-theme');
    expect(themeRadio('Auto')).toBeChecked();
  });

  it('starts from the theme already set on the root element', () => {
    document.documentElement.setAttribute('data-theme', 'light');
    render(<App />);

    expect(themeRadio('Light')).toBeChecked();
    expect(screen.getByTestId('showcase-effective-theme')).toBeEmptyDOMElement();
  });

  it('opens a modal window from the Modal specimen and closes it with Escape', async () => {
    const user = userEvent.setup();
    render(<App />);

    const opener = screen.getByRole('button', { name: 'Small: confirmation' });
    await user.click(opener);
    const dialog = screen.getByRole('dialog', { name: 'Remove host?' });
    expect(dialog).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
  });

  it('reports demo actions in the top bar', async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getByTestId('showcase-last-action')).toHaveTextContent('Last action: none');
    await user.dblClick(screen.getByTestId('host-card-db-primary'));
    expect(screen.getByTestId('host-card-db-primary')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('showcase-last-action')).toHaveTextContent('Last action: Open db-primary');
  });
});
