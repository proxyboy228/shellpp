import { describe, expect, it } from 'vitest';
import { parseQuickConnect } from './quickConnect';

describe('parseQuickConnect (UI 2.1, D23)', () => {
  it.each([
    ['db1.example.com', { address: 'db1.example.com' }],
    ['192.0.2.4', { address: '192.0.2.4' }],
    ['localhost', { address: 'localhost' }],
    ['2001:db8::5', { address: '2001:db8::5' }],
    ['admin@web', { address: 'web', username: 'admin' }],
    ['admin@web.example.org', { address: 'web.example.org', username: 'admin' }],
    ['admin@web.example.org:2222', { address: 'web.example.org', username: 'admin', port: 2222 }],
    ['root@[2001:db8::7]:2200', { address: '2001:db8::7', username: 'root', port: 2200 }],
    ['ssh admin@198.51.100.3 -p 2022', { address: '198.51.100.3', username: 'admin', port: 2022 }],
    ['ssh -p 2022 admin@198.51.100.3', { address: '198.51.100.3', username: 'admin', port: 2022 }],
    ['ssh backup', { address: 'backup' }],
  ])('recognizes %j', (text, target) => {
    expect(parseQuickConnect(text)).toEqual(target);
  });

  it.each([
    [''],
    ['prod'],
    ['web server'],
    ['admin@'],
    ['@host.example.com'],
    ['admin@web.example.org:99999'],
    ['ssh admin@web.example.org -p'],
    ['ssh -v admin@web.example.org'],
    ['ssh admin@a.example.com admin@b.example.com'],
    ['admin@web.example.org:22 extra'],
  ])('treats %j as a search query', (text) => {
    expect(parseQuickConnect(text)).toBeNull();
  });
});
