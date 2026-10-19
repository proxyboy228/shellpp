import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Select, type SelectOption } from './Select';
import css from './Select.module.css?raw';

const OPTIONS: readonly SelectOption<'1' | '15' | 'never'>[] = [
  { value: '1', label: '1 minute' },
  { value: '15', label: '15 minutes' },
  { value: 'never', label: 'Never', disabled: true },
];

function Controlled({ onChange }: { onChange: (value: string) => void }) {
  const [value, setValue] = useState<'1' | '15' | 'never'>('15');
  return (
    <Select
      label="Auto-lock"
      description="Without input"
      options={OPTIONS}
      value={value}
      onValueChange={(next) => {
        setValue(next);
        onChange(next);
      }}
      data-testid="security-auto-lock"
    />
  );
}

describe('Select', () => {
  it('renders a labelled native select with its options and description', () => {
    render(<Controlled onChange={vi.fn()} />);
    const select = screen.getByRole('combobox', { name: 'Auto-lock' });
    expect(select).toBe(screen.getByTestId('security-auto-lock'));
    expect(select).toHaveValue('15');
    expect(select).toHaveAccessibleDescription('Without input');
    expect(screen.getAllByRole('option')).toHaveLength(3);
    expect(screen.getByRole('option', { name: 'Never' })).toBeDisabled();
  });

  it('reports the chosen value', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled onChange={onChange} />);
    await user.selectOptions(screen.getByTestId('security-auto-lock'), '1');
    expect(onChange).toHaveBeenCalledWith('1');
    expect(screen.getByTestId('security-auto-lock')).toHaveValue('1');
  });

  it('shows an error under the field with the derived test id', () => {
    render(
      <Select options={OPTIONS} value="1" onValueChange={vi.fn()} label="Auto-lock" error="Not valid" data-testid="field" />,
    );
    expect(screen.getByTestId('field')).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByTestId('field-error')).toHaveTextContent('Not valid');
    expect(screen.getByTestId('field')).toHaveAccessibleDescription('Not valid');
  });

  it('can be disabled', () => {
    render(<Select options={OPTIONS} value="1" onValueChange={vi.fn()} label="Auto-lock" disabled />);
    expect(screen.getByRole('combobox')).toBeDisabled();
  });

  it('uses only defined tokens', () => {
    const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
    const used = Array.from(css.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
  });
});
