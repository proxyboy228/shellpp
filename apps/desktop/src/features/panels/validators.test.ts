import { describe, expect, it } from 'vitest';
import {
  dedupeTags,
  isIpv6Literal,
  parseAddress,
  parseCharset,
  parseEnv,
  parseInteger,
  parseSecret,
  parseString,
  parseTag,
  parseTags,
  parseText,
} from './validators';

describe('parseAddress', () => {
  it.each([
    ['db1.example.com', 'db1.example.com'],
    ['  web.example.org  ', 'web.example.org'],
    ['192.0.2.10', '192.0.2.10'],
    ['2001:db8::1', '2001:db8::1'],
    ['[2001:db8::1]', '2001:db8::1'],
    ['fe80::1%eth0', 'fe80::1%eth0'],
    ['::ffff:198.51.100.7', '::ffff:198.51.100.7'],
    ['router', 'router'],
  ])('accepts %j', (raw, expected) => {
    expect(parseAddress(raw)).toEqual({ ok: true, value: expected });
  });

  it.each([
    ['', 'required'],
    ['   ', 'required'],
    ['user@example.com', 'invalid'],
    ['host name.example.com', 'invalid'],
    ['example.com/path', 'invalid'],
    ['example.com\\x', 'invalid'],
    ['example.com:22', 'invalid'],
    ['2001:db8::1::2', 'invalid'],
    ['[]', 'invalid'],
    ['a\u0007b', 'invalid'],
    [`${'a'.repeat(256)}`, 'too_long'],
  ])('refuses %j with %s', (raw, reason) => {
    expect(parseAddress(raw)).toEqual({ ok: false, reason });
  });

  it('recognizes IPv6 literals', () => {
    expect(isIpv6Literal('::')).toBe(true);
    expect(isIpv6Literal('1:2:3:4:5:6:7:8')).toBe(true);
    expect(isIpv6Literal('1:2:3:4:5:6:7')).toBe(false);
    expect(isIpv6Literal('fe80::1%')).toBe(false);
    expect(isIpv6Literal('g::1')).toBe(false);
  });
});

describe('scalar fields', () => {
  it('trims and normalizes strings, an empty value is absent', () => {
    expect(parseString('  prod  ', 256)).toEqual({ ok: true, value: 'prod' });
    expect(parseString('   ', 256)).toEqual({ ok: true, value: undefined });
    expect(parseString('', 256, true)).toEqual({ ok: false, reason: 'required' });
    expect(parseString('é', 256)).toEqual({ ok: true, value: 'é' });
    expect(parseString('a\tb', 256)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseString('\u{1F600}'.repeat(3), 3)).toEqual({ ok: true, value: '\u{1F600}'.repeat(3) });
    expect(parseString('abcd', 3)).toEqual({ ok: false, reason: 'too_long' });
  });

  it('keeps line breaks and tabs in text and counts bytes', () => {
    expect(parseText('line 1\r\nline\t2', 100)).toEqual({ ok: true, value: 'line 1\nline\t2' });
    expect(parseText('a\u0000', 100)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseText('éé', 3)).toEqual({ ok: false, reason: 'too_long' });
  });

  it('keeps secrets exactly as typed', () => {
    expect(parseSecret('  pass word ', 4096)).toEqual({ ok: true, value: '  pass word ' });
    expect(parseSecret('a\u0000b', 4096)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseSecret('x'.repeat(4097), 4096)).toEqual({ ok: false, reason: 'too_long' });
  });

  it('parses ports and keepalive ranges', () => {
    expect(parseInteger('', 1, 65535)).toEqual({ ok: true, value: undefined });
    expect(parseInteger(' 2222 ', 1, 65535)).toEqual({ ok: true, value: 2222 });
    expect(parseInteger('0', 1, 65535)).toEqual({ ok: false, reason: 'out_of_range' });
    expect(parseInteger('65536', 1, 65535)).toEqual({ ok: false, reason: 'out_of_range' });
    expect(parseInteger('22a', 1, 65535)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseInteger('-1', 0, 3600)).toEqual({ ok: false, reason: 'invalid' });
    expect(parseInteger('0', 0, 3600)).toEqual({ ok: true, value: 0 });
  });

  it('accepts the offered charsets in lowercase', () => {
    expect(parseCharset('UTF-8')).toEqual({ ok: true, value: 'utf-8' });
    expect(parseCharset('shift_jis')).toEqual({ ok: true, value: 'shift_jis' });
    expect(parseCharset('bad charset')).toEqual({ ok: false, reason: 'invalid' });
  });
});

describe('tags', () => {
  it('refuses commas and long tags', () => {
    expect(parseTag('a,b')).toEqual({ ok: false, reason: 'invalid' });
    expect(parseTag('x'.repeat(65))).toEqual({ ok: false, reason: 'too_long' });
    expect(parseTag(' prod ')).toEqual({ ok: true, value: 'prod' });
  });

  it('de-duplicates case-insensitively and keeps the first spelling', () => {
    expect(dedupeTags(['Prod', 'db', 'prod', 'DB'])).toEqual(['Prod', 'db']);
    expect(parseTags(['Prod', 'prod'])).toEqual({ ok: true, value: ['Prod'] });
    expect(parseTags([])).toEqual({ ok: true, value: undefined });
  });

  it('allows at most 32 tags', () => {
    const tags = Array.from({ length: 33 }, (_, index) => `tag${index}`);
    expect(parseTags(tags)).toEqual({ ok: false, errors: [{ path: 'tags', reason: 'too_many' }] });
  });
});

describe('environment variables', () => {
  it('skips empty rows and validates names', () => {
    expect(
      parseEnv([
        { name: 'LANG', value: 'en_US.UTF-8' },
        { name: '', value: '' },
      ]),
    ).toEqual({ ok: true, value: [{ name: 'LANG', value: 'en_US.UTF-8' }] });
    expect(parseEnv([])).toEqual({ ok: true, value: undefined });
  });

  it('reports every row error with its path', () => {
    expect(
      parseEnv([
        { name: 'A', value: '1' },
        { name: 'A', value: '2' },
        { name: '1BAD', value: '3' },
        { name: '', value: 'orphan' },
      ]),
    ).toEqual({
      ok: false,
      errors: [
        { path: 'ssh.env.1.name', reason: 'duplicate' },
        { path: 'ssh.env.2.name', reason: 'invalid' },
        { path: 'ssh.env.3.name', reason: 'required' },
      ],
    });
  });
});
