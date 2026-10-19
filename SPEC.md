# Project: Shell++, a self-hosted SSH client

## Documentation contents
- `SPEC.md` (this file): goals, principles, stack, functionality, work order.
- `docs/ui-behavior-spec.md`: interface behavior, screen by screen.
- `docs/design-system.md`: palette, typography, tokens, terminal themes.
- `docs/decisions.md`: design decisions accepted before stage 0 and later.
- `docs/architecture.md`, `docs/data-model.md`, `docs/storage-format.md`,
  `docs/sync-protocol.md`, `docs/security.md`: stage 0 design documents
  (architecture, data schema, storage format, sync protocol, threat model). [decision]
- `docs/ru/`: Russian versions of the specifications.

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
  MPL-2.0, OFL for fonts; also Zlib / BSL-1.0 / Unicode-3.0 / CC0-1.0 /
  Unlicense / 0BSD / CDLA-Permissive-2.0 and public domain, for example SQLite).
  Before adding a dependency, check its license and record it in
  THIRD_PARTY_NOTICES.md. [decision]
- System components that are not shipped (the OS graphics and windowing
  libraries, glibc) are platform dependencies. LGPL libraries bundled inside
  an AppImage are listed in THIRD_PARTY_NOTICES.md. GPL and AGPL code is never
  shipped in the client. [decision]
- Development-only dependencies that are never shipped (build, lint, and test
  tools) may also use BlueOak-1.0.0 and CC-BY-4.0. CI checks licenses with
  cargo-deny. [decision]

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
- Desktop UI: GPUI Kit (crate `gpui-kit`: GPUI from the Zed editor plus the
  gpui-component library), a native Rust UI without a webview. [decision]
- Terminal: alacritty_terminal (VT parser, grid, scrollback) + our own GPUI
  element for rendering and input. [decision]
- SSH/SFTP: russh + russh-sftp, key handling via ssh-key.
- Local storage: SQLite. The entire payload of every record is encrypted;
  only the record id, coarse type, vault id, revision, key epoch, and sync
  service fields stay in the clear. Search and sorting run in memory after
  unlock. [decision]
- Unlocking with the master password, optionally with biometrics. The "remember
  on this device" option (on by default since D29, with a note about the risk)
  stores a device key in the OS keychain (crate keyring) that wraps the vault
  key a second time, so the password is not asked at launch; lock and auto-lock
  still ask for it. Without an OS keychain this option is unavailable; there is
  no fallback to a file. [decision]
- Cryptography: Argon2id (KDF), HKDF (key separation), XChaCha20-Poly1305
  (data), X25519 in HPKE (RFC 9180) Auth mode (wrapping of shared vault keys).
  Only vetted crates (RustCrypto, dalek); russh uses the ring backend as an
  explicit exception. The `rsa` crate (advisory RUSTSEC-2023-0071) is a
  documented exception: signing happens locally, and upstream is tracked.
  No homemade cryptography. [decision]
- Interface icons: Lucide (ISC).
- Sync server: Rust (axum) + PostgreSQL, SQLite for small
  installations. Docker image, docker-compose.yml, configuration via env.
- Monorepo:
  - apps/desktop: GPUI Kit application (crate `shellpp-desktop`) [decision]
  - crates/core: data model, encryption, sync client (reused on mobile)
  - crates/ssh: SSH, SFTP, port forwarding, agent
  - crates/term-local: local PTY and serial connections (desktop only) [decision]
  - server/: sync server
  - docs/: specifications, architecture, data schema, storage format, sync protocol,
    threat model, decisions [decision]
- Mobile client (iOS/Android): a separate UI, built later. The logic stays in
  crates/core and crates/ssh, which have no GPUI dependency, so the core
  architecture does not get in the way of this. [decision]

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
  Keys referenced by IdentityFile are imported into the Keychain. [decision]
- PuTTY sessions from the Windows registry, CSV (columns: label, address, port,
  username, group, tags), native JSON, known_hosts. Hashed known_hosts entries
  are kept as they are; `@revoked` entries become revoked keys, and
  `@cert-authority` entries are skipped and counted. [decision]
- Import is idempotent: running it again does not create duplicates. The key
  is the source key (source type plus Host alias, PuTTY session name, or UUID),
  or (address, port, username) when there is none; existing records are never
  overwritten. [decision]

### Sync and team access
- Account on a self-hosted server; the server URL is set by the user.
  Multiple devices are supported.
- Key chain: master password → Argon2id (a random salt bound to the account
  email, versioned parameters) → master key → HKDF → two independent keys:
  the KEK and the auth key. The vault key is generated randomly and stored
  wrapped with the KEK. Each record is encrypted separately. Only the auth key
  goes to the server, which stores a hash of it; a pre-login endpoint returns
  the random salt and the KDF parameters (a deterministic fake salt for an
  unknown email). A password change rewraps the vault key, atomically
  replaces the auth key and the salt, and revokes the tokens of the other
  devices. [decision]
- Protocol:
  - pull/push of changes by cursor (server-side sequence number), one cursor
    per vault; [decision]
  - a record has a UUIDv7, a revision, and a tombstone for deletions;
    tombstones are kept for 90 days, and a device with an older cursor does
    a full resync; [decision]
  - the AEAD associated data binds the vault id, record id, type, revision,
    and key epoch, so substituted or reordered blobs are detected; rollback
    and deletion by the server are not detected; [decision]
  - a push carries `base_revision`; on a mismatch the server rejects it and
    returns the current version; [decision]
  - conflicts are resolved by last-writer-wins at the record level: the winner
    is chosen by a hybrid logical clock inside the record, ties are broken by
    device id, and delete versus edit follows the same rule. The losing
    version is kept as a separate conflict record in the same vault, visible
    on all devices, for manual resolution in the UI. [decision]
- The user chooses which data types to sync. Command history
  and the connection log are not synced by default. The choice is per device,
  applies to both push and pull, and turning a type off does not delete it
  from the server. Known hosts, workspaces, custom themes, and highlighting
  rules sync in the personal vault (on by default); app settings stay local;
  team vaults always sync in full. [decision]
- The connection log is always written to the personal vault. A team vault
  shows only the entries of members who turned on publishing; this is an
  append-only type that viewers can also write. [decision]
- Signing in to an existing account from a device with local data
  re-encrypts the local records under the account's key and merges them with
  deduplication, as an import does. [decision]
- Shared vaults for a team:
  - the vault is the only unit of sharing: sharing a host or snippet moves or
    copies it into a team vault, where it is re-encrypted and gets a new
    record id; [decision]
  - the user has an X25519 key pair whose private part is synced inside the
    personal vault, and a separate Ed25519 SSH ID whose public part is
    published to the server. A host in a team vault may have no credentials:
    each member binds their own SSH ID or identity to it, and the binding is
    stored in the member's personal vault; [decision]
  - the vault key is wrapped with each member's public key (HPKE, Auth mode);
    members' keys are pinned on first use, and their fingerprints are shown
    for comparison outside the server; [decision]
  - invitations take two steps: the link carries only a one-time id, the owner
    confirms the new member by fingerprint, and only then does the owner's
    client wrap the vault key; [decision]
  - roles owner, editor, and viewer; the server enforces them on push, and
    records are not signed in the MVP. Every member, viewers included,
    receives the vault's secrets. A vault can have several owners; deleting
    the account of a vault's only owner is blocked until ownership is
    transferred; [decision]
  - when a member is removed, the vault key is rotated for the remaining members:
    a new key epoch starts, and the owner's client re-encrypts the records in
    the background; [decision]
  - the local copy of a removed member is NOT erased remotely (principle 3);
    it stays as a read-only local archive that can be exported or copied to
    the personal vault. [decision]
- Recovery key: 256 random bits. It wraps the personal vault key and yields a
  recovery auth key that allows setting a new password. In local mode no
  recovery key is shown or required (D29): an honest warning in the UI says
  that a forgotten master password cannot be recovered. The recovery key is
  shown when a sync account is registered. [decision]
- Server:
  - registration is open or invite-only (configurable);
  - TOTP 2FA (reset only with the admin CLI); [decision]
  - admin CLI;
  - health endpoint, rate limiting;
  - SMTP is optional: without it, invitations work only through links; [decision]
  - TLS via reverse proxy (example Caddy configuration);
  - DB backup instructions.

## UI/UX
- Screen structure, navigation, scenarios, keyboard shortcuts, and responses to actions
  are taken from `docs/ui-behavior-spec.md`.
- Appearance (colors, fonts, radii, spacing) is taken from `docs/design-system.md`.
- The **[decision]** mark in the UI specification and in this file means the default
  choice (the decisions behind the marks in this file are recorded in `docs/decisions.md`).
  If it gets in the way of the architecture or contradicts another place in the specification,
  say so before implementation.
- Acceptance: for each screen and scenario in the UI specification there are GPUI tests
  (`TestAppContext` / `VisualTestContext`, headless, no window is opened) that verify the
  described behavior, plus screenshots of our application's window that are checked
  against the UI specification and the design system. A screen is considered done when
  its tests pass and its screenshots match. In tests, native dialogs, the clipboard, and
  launching external programs are replaced by mocks behind a cargo feature that never
  reaches release builds. [decision]

## Security
- Secrets never end up in logs, in plaintext on disk, or in URLs. The allowed
  exceptions are explicit user actions (plaintext export, .txt log export, SFTP
  temporary files), which show a warning; temporary files are deleted when the
  file is closed and when the app exits. [decision]
- The master password is required from the first launch, even without an
  account, and later becomes the account password. It can be changed in local
  mode too. [decision]
- Auto-lock on inactivity. The clipboard is cleared after copying
  a password or key.
- Threat model in docs/security.md.
- Mandatory requirements from `docs/ui-behavior-spec.md`:
  - 4.2: history does not record input during a password prompt, strips control
    sequences, and does not suggest secret-like strings as snippets;
  - 7.3: the private key is hidden by default and shown only via an explicit button;
  - 8.4: port forwarding listens only on 127.0.0.1 by default;
  - 10.2: a host key change blocks the connection until explicit confirmation;
  - 12.4 and 12.6: importing history from the server (12.4) and recording session
    output (12.6) are off by default.

## Performance
Targets, checked by a benchmark:
- startup under 1 s (from launch to the first frame of the main window);
- idle with 300 hosts under 150 MB RSS (the working set on Windows);
- at most 10 MB per new session;
- default scrollback of 10,000 lines.

## Quality
- Unit tests for crates/core: encryption, merge during sync,
  ssh_config parser, secret filter in history.
- Integration tests of SSH/SFTP/port forwarding against OpenSSH in Docker,
  in the Linux CI job; Windows e2e tests use a test SSH/SFTP server built on
  russh and kept in the repository. [decision]
- Sync E2E test: two clients + server.
- Interface tests based on the UI specification (GPUI tests, see "UI/UX"); they find
  elements by element id, not by text. [decision]
- Tests and the screenshot script use a temporary data directory (`SHELLPP_DATA_DIR`)
  and never touch the user's permanent profile.
- Automated contrast test for color tokens.
- CI on GitHub Actions: clippy, rustfmt, tests, builds for Windows, macOS, and Linux.
- Code, comments, commits, and documentation in English. The English
  specifications are canonical; their Russian versions in docs/ru/ are updated
  in the same commit. The interface is in English, with all strings in an i18n
  catalog (en). Stage reports are in Russian. [decision]

## Work order
Work strictly stage by stage. After each stage: tests are green, commit,
brief report (what was done, what remains, what risks there are). Then wait
for my confirmation.

Elements of future stages are hidden behind a stage flag, not shown disabled.
The exception is the window frame and the sidebar: the SFTP tab and all six
sidebar sections are always shown, and one whose screen is not implemented yet
opens an "In development" placeholder. D29 adds two more: every field of the
host panel (UI spec, 3.1, including Mosh and Add Telnet) is shown and its value
is saved before the feature works in sessions, and "Restore from sync" on the
unlock screen is shown disabled until sync exists. The UI tests of a feature are
written in the stage that implements it, and every stage report lists what is
still hidden or shown as a placeholder. [decision]

Stages 0–2 were first built on Tauri + React; their UI is rebuilt on GPUI Kit
(`docs/decisions.md`, 27). [decision]

0. Design documents in docs/: architecture, data schema, storage format,
   sync protocol, threat model (`docs/architecture.md`, `docs/data-model.md`,
   `docs/storage-format.md`, `docs/sync-protocol.md`, `docs/security.md`).
   Plus the theme and base components per design-system.md in `apps/desktop`,
   with a showcase page. Tests for this stage: the contrast test and component
   tests. Show them before writing the main code. [decision]
1. Monorepo skeleton, GPUI Kit application, main window shell (UI spec,
   section 1), encrypted storage, master password, Hosts screen and host
   panel (sections 2–3).
2. SSH terminal: password and key authentication, known hosts and TOFU,
   tabs, splits, themes, terminal sidebar, new tab page
   (sections 4, 6, 10). Also: local terminal, system ssh-agent / Pageant,
   agent forwarding, OS detection with the default script, and a short path
   for adding a key in the host panel; autoreconnect, keepalive, and TERM are
   stored without a settings screen. [decision]
3. Keychain, import (the dialog with all formats from the "Import" section),
   the built-in agent, export to JSON and ssh config (section 7). [decision]
4. SFTP (section 5).
5. Port forwarding, jump hosts, proxy, snippets (sections 8, 9).
6. Logs, settings, keyboard shortcuts, keyword highlighting (sections 11, 12). [decision]
7. Sync server and sync client, multiple devices.
8. Shared vaults, roles, invitations, sharing of hosts and snippets.
9. Polish: workspaces, broadcast, Telnet and serial connections, auto-update
   that can be turned off, from my own source (the automatic check is off by
   default, a manual check is always available, and the source can be
   overridden in Settings → Updates). [decision]
10. Later stages: mobile apps, shared session (multiplayer),
    import from cloud providers, autocomplete, snippet generation via
    a pluggable LLM, Mosh (a separate stage; until then its toggle in the
    host panel only saves the value, D29). [decision]

If something in the specification is contradictory or technically unsound, say
so before implementation, rather than silently.
