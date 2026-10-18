import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { PasswordInput } from './PasswordInput';
import css from './PasswordInput.module.css?raw';

function tokenNames(source: string): Set<string> {
  return new Set(Array.from(source.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
}

function usedTokens(source: string): string[] {
  return Array.from(source.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
}

describe('PasswordInput', () => {
  it('is masked by default and offers a show toggle', () => {
    render(<PasswordInput label="Password" data-testid="password" />);

    const input = screen.getByLabelText('Password');
    expect(input).toHaveAttribute('type', 'password');
    expect(screen.getByTestId('password')).toBe(input);

    const toggle = screen.getByRole('button', { name: 'Show password' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByTestId('password-toggle')).toBe(toggle);
  });

  it('turns off autocomplete, spell check and autocorrection by default', () => {
    render(<PasswordInput label="Password" />);

    const input = screen.getByLabelText('Password');
    expect(input).toHaveAttribute('autocomplete', 'off');
    expect(input).toHaveAttribute('spellcheck', 'false');
    expect(input).toHaveAttribute('autocapitalize', 'off');
    expect(input).toHaveAttribute('autocorrect', 'off');
  });

  it('lets the caller choose autocomplete and forwards the ref', () => {
    const ref = createRef<HTMLInputElement>();
    render(<PasswordInput ref={ref} label="Master password" autoComplete="current-password" />);

    const input = screen.getByLabelText('Master password');
    expect(input).toHaveAttribute('autocomplete', 'current-password');
    expect(ref.current).toBe(input);
  });

  it('shows and hides the value with the toggle and keeps what was typed', async () => {
    const user = userEvent.setup();
    render(<PasswordInput label="Password" />);

    const input = screen.getByLabelText('Password');
    await user.type(input, 's3cret');

    await user.click(screen.getByRole('button', { name: 'Show password' }));
    expect(input).toHaveAttribute('type', 'text');
    expect(screen.getByRole('button', { name: 'Show password' })).toHaveAttribute('aria-pressed', 'true');
    expect(input).toHaveValue('s3cret');

    await user.click(screen.getByRole('button', { name: 'Show password' }));
    expect(input).toHaveAttribute('type', 'password');
    expect(input).toHaveValue('s3cret');
  });

  it('reaches the toggle with Tab and flips it with Space and Enter', async () => {
    const user = userEvent.setup();
    render(<PasswordInput label="Passphrase" toggleLabel="Show passphrase" />);

    const input = screen.getByLabelText('Passphrase');
    await user.tab();
    expect(input).toHaveFocus();
    await user.tab();
    const toggle = screen.getByRole('button', { name: 'Show passphrase' });
    expect(toggle).toHaveFocus();

    await user.keyboard('[Space]');
    expect(input).toHaveAttribute('type', 'text');
    await user.keyboard('{Enter}');
    expect(input).toHaveAttribute('type', 'password');
  });

  it('supports controlled visibility', async () => {
    const user = userEvent.setup();
    const onVisibleChange = vi.fn();

    function Controlled() {
      const [visible, setVisible] = useState(false);
      return (
        <PasswordInput
          label="Password"
          visible={visible}
          onVisibleChange={(next) => {
            onVisibleChange(next);
            setVisible(next);
          }}
        />
      );
    }

    const { rerender } = render(<PasswordInput label="Fixed" visible={false} onVisibleChange={onVisibleChange} />);
    await user.click(screen.getByRole('button', { name: 'Show password' }));
    expect(onVisibleChange).toHaveBeenLastCalledWith(true);
    expect(screen.getByLabelText('Fixed')).toHaveAttribute('type', 'password');

    rerender(<Controlled />);
    await user.click(screen.getByRole('button', { name: 'Show password' }));
    expect(screen.getByLabelText('Password')).toHaveAttribute('type', 'text');
  });

  it('starts visible with defaultVisible', () => {
    render(<PasswordInput label="Password" defaultVisible />);

    expect(screen.getByLabelText('Password')).toHaveAttribute('type', 'text');
  });

  it('disables the field and the toggle together', async () => {
    const user = userEvent.setup();
    render(<PasswordInput label="Password" disabled />);

    const toggle = screen.getByRole('button', { name: 'Show password' });
    expect(screen.getByLabelText('Password')).toBeDisabled();
    expect(toggle).toBeDisabled();
    await user.click(toggle);
    expect(screen.getByLabelText('Password')).toHaveAttribute('type', 'password');
  });

  it('shows an error linked to the field', () => {
    render(<PasswordInput label="Master password" error="Wrong password" />);

    const input = screen.getByLabelText('Master password');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription('Wrong password');
  });
});

describe('PasswordInput styles', () => {
  it('uses only variables defined in tokens.css and no raw colors or durations', () => {
    const defined = tokenNames(tokensCss);
    expect(usedTokens(css).filter((name) => !defined.has(name))).toEqual([]);
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(?:rgba?|hsla?)\(/);
    expect(css).not.toMatch(/\b\d+(?:\.\d+)?m?s\b/);
  });

  it('never applies hover or press styles to a disabled toggle', () => {
    for (const match of css.matchAll(/^[^{}]*:(?:hover|active)[^{]*\{/gm)) {
      expect(match[0]).toContain(':not(:disabled)');
    }
  });
});
