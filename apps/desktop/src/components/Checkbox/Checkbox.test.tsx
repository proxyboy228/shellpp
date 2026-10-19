import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import tokensCss from '../../theme/tokens.css?raw';
import { Checkbox } from './Checkbox';
import css from './Checkbox.module.css?raw';

function Controlled({ onChange }: { onChange: (checked: boolean) => void }) {
  const [checked, setChecked] = useState(false);
  return (
    <Checkbox
      checked={checked}
      onCheckedChange={(next) => {
        setChecked(next);
        onChange(next);
      }}
      label="I saved the recovery key"
      data-testid="first-run-recovery-saved"
    />
  );
}

describe('Checkbox', () => {
  it('is a labelled native checkbox that the label toggles', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled onChange={onChange} />);
    const box = screen.getByRole('checkbox', { name: 'I saved the recovery key' });
    expect(box).toBe(screen.getByTestId('first-run-recovery-saved'));
    expect(box).not.toBeChecked();
    await user.click(screen.getByText('I saved the recovery key'));
    expect(box).toBeChecked();
    expect(onChange).toHaveBeenLastCalledWith(true);
    await user.keyboard(' ');
    expect(box).not.toBeChecked();
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it('can be disabled', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Checkbox checked={false} onCheckedChange={onChange} label="Saved" disabled />);
    await user.click(screen.getByRole('checkbox'));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole('checkbox')).toBeDisabled();
  });

  it('uses only defined tokens', () => {
    const defined = new Set(Array.from(tokensCss.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]));
    const used = Array.from(css.matchAll(/var\((--[\w-]+)/g), (match) => match[1]);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
  });
});
