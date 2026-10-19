// Demo fixtures for the mock (`fixtures: 'demo'`): nested groups with
// inherited SSH settings and hosts with tags and usernames. Addresses are
// documentation-only (example.com, example.org, 192.0.2.0/24,
// 198.51.100.0/24, 203.0.113.0/24, 2001:db8::/32).
//
// Owner: W4.

import type { RecordStore } from './records';

/** A throwaway password for the demo host, made at run time. */
function demoSecret(random: () => number): string {
  return `demo-${random().toString(36)}${random().toString(36)}`;
}

export function seedDemoRecords(store: RecordStore, random: () => number): void {
  const production = store.insert('group', {
    name: 'Production',
    color: 'coral',
    ssh: { username: 'deploy', port: 2222 },
  });
  const databases = store.insert('group', {
    name: 'Databases',
    parent_id: production.record_id,
    color: 'ocean',
    ssh: { port: 2200 },
  });
  const staging = store.insert('group', { name: 'Staging', color: 'amber', ssh: { username: 'ubuntu' } });

  store.insert('host', {
    address: '203.0.113.10',
    label: 'web-1',
    group_id: production.record_id,
    tags: ['web', 'prod'],
    color: 'jade',
    ssh: {},
  });
  store.insert('host', {
    address: '203.0.113.11',
    label: 'web-2',
    group_id: production.record_id,
    tags: ['web', 'prod'],
    color: 'jade',
    ssh: {},
  });
  store.insert('host', {
    address: 'db1.example.com',
    label: 'DB primary',
    group_id: databases.record_id,
    tags: ['db', 'prod'],
    color: 'ocean',
    ssh: { username: 'postgres' },
  });
  store.insert('host', {
    address: 'app.staging.example.org',
    group_id: staging.record_id,
    tags: ['web', 'staging'],
    ssh: {},
  });
  store.insert(
    'host',
    {
      address: '198.51.100.7',
      label: 'Bastion',
      tags: ['jump'],
      notes: 'Entry point for the documentation network.',
      ssh: { username: 'admin', agent_forwarding: true },
    },
    { 'ssh.password': demoSecret(random) },
  );
  store.insert('host', { address: '2001:db8::10', ssh: { port: 2022 } });
  store.insert('host', { address: '192.0.2.44', label: 'printer', color: 'plum', ssh: {} });
}
