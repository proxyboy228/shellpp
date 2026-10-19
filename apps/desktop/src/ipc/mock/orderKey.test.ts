// Manual order keys (docs/data-model.md, section 18.1).

import { describe, expect, it } from 'vitest';
import { compareOrderKeys, evenlySpacedKeys, orderKeyBetween } from './orderKey';

describe('orderKeyBetween', () => {
  it('returns the midpoint with the fewest digits', () => {
    expect(orderKeyBetween(null, null)).toBe('V');
    expect(orderKeyBetween('V', null)).toBe('k');
    expect(orderKeyBetween(null, 'V')).toBe('F');
    expect(orderKeyBetween('A', 'B')).toBe('AV');
    expect(orderKeyBetween('A1', 'A2')).toBe('A1V');
    expect(orderKeyBetween(null, '1')).toBe('0V');
  });

  it('stays strictly between its neighbours and never ends in 0', () => {
    let low: string | null = null;
    let high: string | null = 'V';
    for (let step = 0; step < 40; step += 1) {
      const key = orderKeyBetween(low, high);
      expect(key).not.toBeNull();
      const value = key as string;
      expect(value.endsWith('0')).toBe(false);
      if (low !== null) {
        expect(compareOrderKeys(low, value)).toBe(-1);
      }
      if (high !== null) {
        expect(compareOrderKeys(value, high)).toBe(-1);
      }
      if (step % 2 === 0) {
        low = value;
      } else {
        high = value;
      }
    }
  });

  it('gives up beyond 64 characters', () => {
    const low = '0'.repeat(63) + '1';
    const high = '0'.repeat(63) + '2';
    expect(orderKeyBetween(low, high)).toBeNull();
  });
});

describe('evenlySpacedKeys', () => {
  it('returns increasing short keys', () => {
    const keys = evenlySpacedKeys(100);
    expect(keys).toHaveLength(100);
    for (let index = 1; index < keys.length; index += 1) {
      expect(compareOrderKeys(keys[index - 1], keys[index])).toBe(-1);
    }
    expect(Math.max(...keys.map((key) => key.length))).toBeLessThanOrEqual(2);
  });
});
