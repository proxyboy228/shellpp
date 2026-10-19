// Identifiers for the in-browser mock: UUIDv7-shaped record ids, fake
// recovery keys and a seeded pseudo-random generator.
//
// Nothing here is cryptographic: the mock runs only in Vitest and in
// `pnpm dev` without Tauri, and never protects real data.
//
// Owner: W4.

/** Deterministic xorshift32 generator (mock data only, not for secrets). */
export function createRandom(seed = 0x5eed1234): () => number {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
}

function hex(value: number, length: number): string {
  return value.toString(16).padStart(length, '0').slice(-length);
}

/** Creates UUIDv7-shaped ids whose timestamps strictly increase. */
export function createIdFactory(random: () => number, now: () => number): () => { id: string; ms: number } {
  let lastMs = 0;
  return () => {
    const ms = Math.max(Math.floor(now()), lastMs + 1);
    lastMs = ms;
    const timeHex = hex(Math.floor(ms / 0x10000), 8) + hex(ms % 0x10000, 4);
    const randA = hex(random() & 0x0fff, 3);
    const variant = hex(0x8000 | (random() & 0x3fff), 4);
    const randB = hex(random(), 8) + hex(random() & 0xffff, 4);
    const id = `${timeHex.slice(0, 8)}-${timeHex.slice(8, 12)}-7${randA}-${variant}-${randB}`;
    return { id, ms };
  };
}

/** Milliseconds encoded in a UUIDv7 (NaN for other ids). */
export function uuidV7Millis(id: string): number {
  const compact = id.replace(/-/g, '');
  if (compact.length !== 32 || compact[12] !== '7') {
    return Number.NaN;
  }
  return Number.parseInt(compact.slice(0, 12), 16);
}

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/** Lowercase hyphenated UUID text. */
export function isUuid(value: unknown): value is string {
  return typeof value === 'string' && UUID_PATTERN.test(value);
}

/** Crockford base32 alphabet (docs/security.md, section 5.8). */
const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

/** A fake recovery key in the real format: `RK1-` and 11 groups of 5 characters. */
export function fakeRecoveryKey(random: () => number): string {
  const groups: string[] = [];
  for (let group = 0; group < 11; group += 1) {
    let text = '';
    for (let index = 0; index < 5; index += 1) {
      text += CROCKFORD[random() % 32];
    }
    groups.push(text);
  }
  return `RK1-${groups.join('-')}`;
}

/**
 * The 55 key characters of a typed recovery key, or null when it is malformed.
 * Ignores case, spaces, hyphens and an `RK1` prefix; maps I, L to 1 and O to 0.
 */
export function normalizeRecoveryKey(text: string): string | null {
  let compact = text.toUpperCase().replace(/[\s-]/g, '');
  if (compact.startsWith('RK1') && compact.length === 58) {
    compact = compact.slice(3);
  }
  compact = compact.replace(/[IL]/g, '1').replace(/O/g, '0');
  if (compact.length !== 55) {
    return null;
  }
  for (const character of compact) {
    if (!CROCKFORD.includes(character)) {
      return null;
    }
  }
  return compact;
}
