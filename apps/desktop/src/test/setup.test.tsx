// Checks the test toolchain itself: jsdom, jest-dom matchers, user-event,
// CSS modules and the defaults from `setup.ts`.
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import styles from './fixture.module.css';

function Counter() {
  const [count, setCount] = useState(0);
  return (
    <button type="button" onClick={() => setCount((value) => value + 1)}>
      Clicked {count}
    </button>
  );
}

describe('test setup', () => {
  it('registers jest-dom matchers and handles user events', async () => {
    const user = userEvent.setup();
    render(<Counter />);

    const button = screen.getByRole('button', { name: 'Clicked 0' });
    expect(button).toBeInTheDocument();

    await user.click(button);
    expect(button).toHaveTextContent('Clicked 1');
  });

  it('cleans up the document between tests', () => {
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('processes CSS modules', () => {
    render(<div className={styles.marker} data-testid="marker" />);

    expect(styles.marker).toMatch(/marker/);
    expect(screen.getByTestId('marker')).toHaveClass(styles.marker);
    expect(screen.getByTestId('marker')).not.toBeVisible();
  });

  it('provides an inert matchMedia', () => {
    expect(window.matchMedia('(prefers-color-scheme: dark)').matches).toBe(false);
  });
});
