# Project: self-hosted SSH client (working title: Shell++)

## Documentation contents
- `SPEC.md` (this file): goals, principles, stack, functionality, work order.
- `docs/ui-behavior-spec.md`: interface behavior, screen by screen.
- `docs/design-system.md`: palette, typography, tokens, terminal themes.

## Goal
A cross-platform SSH client with functionality on par with commercial clients
(hosts, keys, snippets, SFTP, port forwarding, sync between devices),
but with a different data ownership model. Motivation: in a commercial client my
local data became inaccessible when someone else's team subscription expired.
Here this must be impossible by design.

## Clean room: mandatory rules
- Do not use Termius code, resources, binaries, decompilation, network protocol, or
  storage format. Do not search for or open their sources or dumps.
- Do not use the Termius name, logo, icons, brand colors, illustrations,
  or interface text.
- The only source of interface requirements: `docs/ui-behavior-spec.md`
  and `docs/design-system.md`. Do not request or use screenshots of the original.
  If a description is not sufficient, ask a question instead of filling in the gaps
  based on your own ideas about the original.
- Dependencies with permissive licenses only (MIT / Apache-2.0 / BSD / ISC /
  MPL-2.0, OFL for fonts). Before adding a dependency, check its license
  and record it in THIRD_PARTY_NOTICES.

## Principles (non-negotiable)
1. Local-first. The application works fully offline, without an account or server.
   Sync is optional.
2. No license or subscription checks, trials, pricing tiers, telemetry,
   phone-home, or remote disabling of features.
3. User data is always readable locally. If access to a shared
   vault is revoked, the account is deleted, the user signs out, or the server is unavailable, the local copy
   remains accessible and exportable. Revoking access stops only FUTURE
   sync and never remotely erases or locks data that has already been
   received.
4. Export to open formats at any time: JSON (documented schema,
   plaintext or encrypted), ~/.ssh/config, keys in OpenSSH format,
   known_hosts. The storage format is described in docs/.
5. Zero-knowledge sync. The server stores only encrypted blobs and sync
   metadata. Encryption keys never leave the client in plaintext.

## Default stack
If you see a reason to replace something, propose it BEFORE starting work.
- Desktop: Tauri 2 (Rust core) + React + TypeScript + Vite.
- Terminal: xterm.js (addons: fit, search, webgl, web-links, unicode11).
- SSH/SFTP: russh + russh-sftp, key handling via ssh-key.
- Local storage: SQLite. Secrets are encrypted at the record level.
  The key is stored in the OS keychain (crate keyring). Unlocking with the master password,
  optionally with biometrics.
- Cryptography: Argon2id (KDF), XChaCha20-Poly1305 (data), X25519
  (wrapping of shared vault keys). Only vetted crates (RustCrypto, dalek).
  No homemade cryptography.
- Interface icons: Lucide (ISC).
- Sync server: Rust (axum) + PostgreSQL, SQLite for small
  installations. Docker image, docker-compose.yml, configuration via env.
- Monorepo:
  - apps/desktop: Tauri application
  - crates/core: data model, encryption, sync client (reused on mobile)
  - crates/ssh: SSH, SFTP, port forwarding, agent
  - server/: sync server
  - docs/: specifications, architecture, data format, sync protocol, threat model
- Mobile apps (iOS/Android) are built later via Tauri 2 mobile.
  The core architecture must not get in the way of this.

## Functionality
Interface details for each item are described in `docs/ui-behavior-spec.md`.

### Hosts
- Host fields: label, address, port, identity, group, tags, notes, color,
  startup snippet, environment variables, keepalive, encoding, terminal theme,
  jump host chain (ProxyJump), proxy (HTTP/SOCKS), agent forwarding,
  additional protocols (Telnet, later Mosh).
- Nested groups with settings inheritance (username, identity, port, jump host).
- Identity: username + password and/or key, reused across hosts.
- Search and filter by tags, quick connect from a `user@host:port` string,
  search across hosts and tabs (Ctrl+K).

### Keychain
- Generation of Ed25519, ECDSA, and RSA keys.
- Import of keys in OpenSSH, PEM, and PuTTY .ppk formats, passphrase support.
- Certificates and FIDO2 keys.
- Uploading the public key to a server (similar to ssh-copy-id).
- Working with the system ssh-agent / Pageant, and a built-in agent.
- Known hosts: TOFU with the fingerprint shown on first connection, a blocking
  warning when the host key changes, list management, import and export.

### Terminal
- Tabs, horizontal and vertical splits, tab renaming,
  auto-reconnect on connection drop, workspaces (saved sets of tabs).
- Color themes (import of iTerm2 and Windows Terminal schemes), font, size,
  cursor style.
- Search in output, copy-on-select (option), confirmation of multi-line paste.
- Keyword highlighting in output, including user-defined rules.
- Broadcast of input to multiple sessions.
- Command history and session logging (options, see the "Security" section).
- Local terminal and serial connections.

### Snippets
- Variables `{{var}}`: on run, a form appears for entering the values.
- Packages (folders) and tags.
- Run in the current session or on several target hosts at once.

### Port forwarding
- Local, Remote, and Dynamic (SOCKS5).
- Saved rules, wizard, autostart on connection,
  status indicator.

### SFTP
- Dual-pane manager: local ↔ remote and remote ↔ remote.
- Drag & drop, transfer queue with progress, pause, and cancel.
- chmod, rename, delete, folder creation, hidden files.
- Opening a file in an external editor with automatic upload of changes back.

### Import
- ~/.ssh/config, including Include, Host *, Match (at least basic) and ProxyJump.
- PuTTY sessions from the Windows registry, CSV, native JSON, known_hosts.
- Import is idempotent: running it again does not create duplicates.

### Sync and team access
- Account on a self-hosted server; the server URL is set by the user.
  Multiple devices are supported.
- Key chain: master password → Argon2id → KEK. The vault key is generated
  randomly and stored wrapped with the KEK. Each record is encrypted separately.
- Protocol:
  - pull/push of changes by cursor (server-side sequence number);
  - a record has a UUIDv7, a revision, and a tombstone for deletions;
  - conflicts are resolved by last-writer-wins at the record level; the losing
    version is kept for manual resolution in the UI.
- The user chooses which data types to sync. Command history
  and the connection log are not synced by default.
- Shared vaults for a team:
  - the user has an X25519 key pair;
  - the vault key is wrapped with each member's public key;
  - roles owner, editor, and viewer;
  - when a member is removed, the vault key is rotated for the remaining members;
  - the local copy of a removed member is NOT erased remotely (principle 3).
- Recovery key at registration. An honest warning in the UI: without the password
  and the recovery key, the data cannot be recovered.
- Server:
  - registration is open or invite-only (configurable);
  - TOTP 2FA;
  - admin CLI;
  - health endpoint, rate limiting;
  - TLS via reverse proxy (example Caddy configuration);
  - DB backup instructions.

## UI/UX
- Screen structure, navigation, scenarios, keyboard shortcuts, and responses to actions
  are taken from `docs/ui-behavior-spec.md`.
- Appearance (colors, fonts, radii, spacing) is taken from `docs/design-system.md`.
- The **[decision]** mark in the UI specification means the default choice.
  If it gets in the way of the architecture or contradicts another place in the specification,
  say so before implementation.
- Acceptance: for each screen and scenario in the UI specification there is an e2e test
  (WebdriverIO + tauri-driver) that verifies the described behavior. A screen is considered
  done when its tests pass.

## Security
- Secrets never end up in logs, in plaintext on disk, or in URLs.
- Auto-lock on inactivity. The clipboard is cleared after copying
  a password or key.
- Strict CSP in Tauri and a minimal set of allowed commands.
- Threat model in docs/security.md.
- Mandatory requirements from `docs/ui-behavior-spec.md`:
  - 4.2: history does not record input during a password prompt, strips control
    sequences, and does not suggest secret-like strings as snippets;
  - 7.3: the private key is hidden by default and shown only via an explicit button;
  - 8.4: port forwarding listens only on 127.0.0.1 by default;
  - 10.2: a host key change blocks the connection until explicit confirmation;
  - 12.4: importing history from the server and recording session output are off by default.

## Quality
- Unit tests for crates/core: encryption, merge during sync,
  ssh_config parser, secret filter in history.
- Integration tests of SSH/SFTP/port forwarding against OpenSSH in Docker.
- Sync E2E test: two clients + server.
- Interface E2E tests based on the UI specification.
- Automated contrast test for color tokens.
- CI on GitHub Actions: clippy, eslint, tests, builds for Windows, macOS, and Linux.
- Code, comments, commits, and documentation in English.

## Work order
Work strictly stage by stage. After each stage: tests are green, commit,
brief report (what was done, what remains, what risks there are). Then wait
for my confirmation.

0. Design documents in docs/: architecture, data schema, storage format,
   sync protocol, threat model. Plus a tokens file and base
   components per design-system.md. Show them before writing the main code.
1. Monorepo skeleton, Tauri application, main window shell (UI spec,
   section 1), encrypted storage, master password, Hosts screen and host
   panel (sections 2–3).
2. SSH terminal: password and key authentication, known hosts and TOFU,
   tabs, splits, themes, terminal sidebar, new tab page
   (sections 4, 6, 10).
3. Keychain, ~/.ssh/config import, export to JSON and ssh config (section 7).
4. SFTP (section 5).
5. Port forwarding, jump hosts, proxy, snippets (sections 8, 9).
6. Logs, settings, keyboard shortcuts (sections 11, 12).
7. Sync server and sync client, multiple devices.
8. Shared vaults, roles, invitations, sharing of hosts and snippets.
9. Polish: workspaces, broadcast, auto-update that can be turned off, from my
   own source.
10. Later stages: mobile apps, shared session (multiplayer),
    import from cloud providers, autocomplete, snippet generation via
    a pluggable LLM.

If something in the specification is contradictory or technically unsound, say
so before implementation, rather than silently.
