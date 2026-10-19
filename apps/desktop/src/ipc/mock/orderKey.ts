// Manual order keys for the mock (docs/data-model.md, section 18.1): base-62
// strings `0-9A-Za-z`, 1 to 64 characters, never ending in `0`, compared by
// bytes. A key between two others is their midpoint read as a base-62
// fraction, shortened to the fewest digits that stay strictly between them.
//
// Owner: W4.

export const ORDER_KEY_DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
export const MAX_ORDER_KEY_LENGTH = 64;
const BASE = ORDER_KEY_DIGITS.length;

function digit(key: string, index: number): number {
  const character = key[index];
  return character === undefined ? 0 : ORDER_KEY_DIGITS.indexOf(character);
}

/**
 * The shortest key strictly between `low` and `high` (null = the open end).
 * Returns null when the result would be longer than 64 characters (the
 * caller then rebalances the container).
 */
export function orderKeyBetween(low: string | null, high: string | null): string | null {
  const lowKey = low ?? '';
  let result = '';
  for (let index = 0; index < MAX_ORDER_KEY_LENGTH; index += 1) {
    const lowDigit = digit(lowKey, index);
    const highDigit = high === null || index >= high.length ? (high === null ? BASE : 0) : digit(high, index);
    // Once the prefix is below `high`, the remaining high digits are open.
    const effectiveHigh = high !== null && result < high.slice(0, index) ? BASE : highDigit;
    if (effectiveHigh - lowDigit > 1) {
      const mid = Math.floor((lowDigit + effectiveHigh) / 2);
      return result + ORDER_KEY_DIGITS[mid];
    }
    result += ORDER_KEY_DIGITS[lowDigit];
  }
  return null;
}

/** `count` evenly spaced short keys (rebalancing a container). */
export function evenlySpacedKeys(count: number): string[] {
  const keys: string[] = [];
  let width = 1;
  while (BASE ** width - 1 < count) {
    width += 1;
  }
  const span = BASE ** width;
  for (let index = 1; index <= count; index += 1) {
    let value = Math.floor((index * span) / (count + 1));
    let text = '';
    for (let position = 0; position < width; position += 1) {
      text = ORDER_KEY_DIGITS[value % BASE] + text;
      value = Math.floor(value / BASE);
    }
    keys.push(text.replace(/0+$/, '') || ORDER_KEY_DIGITS[1]);
  }
  return keys;
}

/** Byte order of two keys (ASCII). */
export function compareOrderKeys(a: string, b: string): number {
  if (a === b) {
    return 0;
  }
  return a < b ? -1 : 1;
}
