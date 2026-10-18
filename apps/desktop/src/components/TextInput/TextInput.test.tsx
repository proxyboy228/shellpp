import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Search } from 'lucide-react';
import { createRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { TextInput } from './TextInput';
import css from './TextInput.module.css?raw';
import styles from './TextInput.module.css';

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('TextInput', () => {
  it('renders a text field linked to its visible label', () => {
    render(<TextInput label="Label" placeholder="My server" />);

    const input = screen.getByRole('textbox', { name: 'Label' });
    expect(input).toHaveAttribute('type', 'text');
    expect(input).toHaveAttribute('placeholder', 'My server');
    expect(input).toHaveClass(styles.input);
    expect(input).not.toHaveAttribute('aria-invalid');
    expect(input).not.toHaveAttribute('aria-describedby');
  });

  it('accepts aria-label when there is no visible label', () => {
    render(<TextInput aria-label="Search hosts" type="search" />);

    expect(screen.getByRole('searchbox', { name: 'Search hosts' })).toBeInTheDocument();
  });

  it('forwards data-testid, ref, id and other attributes to the input', () => {
    const ref = createRef<HTMLInputElement>();
    render(
      <TextInput
        ref={ref}
        id="address"
        label="Address"
        data-testid="host-address"
        className="wrapper"
        inputClassName="field"
        autoComplete="off"
        maxLength={10}
      />,
    );

    const input = screen.getByTestId('host-address');
    expect(input).toBe(screen.getByLabelText('Address'));
    expect(input).toHaveAttribute('id', 'address');
    expect(input).toHaveClass('field');
    expect(input).toHaveAttribute('autocomplete', 'off');
    expect(input).toHaveAttribute('maxlength', '10');
    expect(input.closest(`.${styles.root}`)).toHaveClass('wrapper');
    expect(ref.current).toBe(input);
  });

  it('accepts typing when uncontrolled and when controlled', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();

    function Controlled() {
      const [value, setValue] = useState('');
      return <TextInput aria-label="Controlled" value={value} onChange={(event) => setValue(event.target.value)} />;
    }

    render(
      <>
        <TextInput aria-label="Uncontrolled" onChange={onChange} />
        <Controlled />
      </>,
    );

    await user.type(screen.getByRole('textbox', { name: 'Uncontrolled' }), 'web-01');
    expect(screen.getByRole('textbox', { name: 'Uncontrolled' })).toHaveValue('web-01');
    expect(onChange).toHaveBeenCalledTimes(6);

    await user.type(screen.getByRole('textbox', { name: 'Controlled' }), '10.0.0.1');
    expect(screen.getByRole('textbox', { name: 'Controlled' })).toHaveValue('10.0.0.1');
  });

  it('is reached with Tab', async () => {
    const user = userEvent.setup();
    render(<TextInput label="Port" />);

    await user.tab();
    expect(screen.getByRole('textbox', { name: 'Port' })).toHaveFocus();
  });

  it('links the description and the error message', () => {
    render(
      <TextInput
        label="Port"
        description="1 to 65535"
        error="Port must be a number"
        aria-describedby="external"
        data-testid="port"
      />,
    );

    const input = screen.getByRole('textbox', { name: 'Port' });
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription('1 to 65535 Port must be a number');
    expect(input.getAttribute('aria-describedby')?.split(' ')[0]).toBe('external');
    expect(screen.getByTestId('port-error')).toHaveTextContent('Port must be a number');
    expect(input.closest(`.${styles.root}`)).toHaveAttribute('data-invalid', 'true');
  });

  it('marks the field invalid without a message when error is true', () => {
    render(<TextInput label="Port" error data-testid="port" />);

    expect(screen.getByRole('textbox', { name: 'Port' })).toHaveAttribute('aria-invalid', 'true');
    expect(screen.queryByTestId('port-error')).not.toBeInTheDocument();
  });

  it('ignores input and leaves the tab order when disabled', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TextInput label="Username" disabled onChange={onChange} />);

    const input = screen.getByRole('textbox', { name: 'Username' });
    expect(input).toBeDisabled();
    expect(input.closest(`.${styles.root}`)).toHaveAttribute('data-disabled', 'true');

    await user.tab();
    expect(input).not.toHaveFocus();
    await user.type(input, 'root');
    expect(input).toHaveValue('');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('renders the monospace style, a hidden leading icon and a trailing slot', () => {
    render(
      <TextInput
        aria-label="Public key"
        mono
        leadingIcon={<Search data-testid="leading" />}
        trailing={<button type="button">Paste</button>}
      />,
    );

    expect(screen.getByRole('textbox', { name: 'Public key' })).toHaveClass(styles.mono);
    expect(screen.getByTestId('leading').closest('[aria-hidden="true"]')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Paste' })).toBeInTheDocument();
  });
});

describe('TextInput styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('defines the focus ring, the disabled state and reduced motion', () => {
    expect(css).toMatch(
      /:focus-visible\)\s*\{\s*outline: var\(--focus-ring-width\) solid var\(--focus-ring\);\s*outline-offset: var\(--focus-ring-offset\);/,
    );
    expect(css).toMatch(/\[data-disabled\]\s*\{[^}]*opacity: var\(--opacity-disabled\)/);
    expect(css).toContain('color: var(--text-muted)');
    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
  });
});
