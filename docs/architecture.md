# Architecture

This document describes how Shell++ is built: components and their licenses, the process and window model, the main runtime flows, the platform abstraction, stages and stage flags, identifiers, the dependency policy, testing, packaging and server deployment. It is a stage 0 design document; no Rust code exists yet.

> **Note (2026-10-19).** The UI moved from Tauri + React to GPUI Kit ([decisions.md](decisions.md), D27). The sections that describe Tauri, the webview, IPC commands and events, and the CSP are superseded by D27 and will be rewritten; where they differ, D27 and `SPEC.md` win. The stage flag rule is amended by D28 for the window frame and the sidebar.

Marks:

- (D*n*): accepted decision *n* in [decisions.md](decisions.md).
- (stage 0 decision): a choice made while writing the stage 0 documents that is not in the specifications or the decisions. It can be changed through the normal specification change process.
- "UI *n.m*": section *n.m* of [ui-behavior-spec.md](ui-behavior-spec.md).

Related documents:

| Document | Contents |
|---|---|
| [../SPEC.md](../SPEC.md) | goals, principles, stack, work order |
| [ui-behavior-spec.md](ui-behavior-spec.md) | screens and behavior |
| [design-system.md](design-system.md) | tokens, terminal themes, component states |
| [decisions.md](decisions.md) | the 26 decisions taken before stage 0 |
| [data-model.md](data-model.md) | entities, record types and payload fields, references, group inheritance, sync classes |
| [storage-format.md](storage-format.md) | encodings, record envelope, local SQLite schema, files on disk, export and recording formats |
| [sync-protocol.md](sync-protocol.md) | HTTP API, error codes, HLC, conflicts, key epochs, invitations, limits, server schema |
| [security.md](security.md) | threat model, key hierarchy, KDF, lock behavior, server-visible metadata |

This document owns: components and crates, the process and window model, platform traits (including the secrets trait), identifiers, directories, the dependency policy, the stage mapping, the test strategy, the updater, and server deployment and configuration. Other documents link here for these topics.

## 1. Goals and constraints

### 1.1 Principles and their consequences

| Principle (SPEC) | Consequence for the architecture |
|---|---|
| 1. Local-first | Every feature works against the local store in `shellpp-core`. The sync client is an optional module that stays idle without a bound account. No screen waits for the network to show or edit data. |
| 2. No checks, no telemetry | The client makes network requests only for connections the user starts (SSH, SFTP, Telnet), for sync with a server the user configured, and for update checks (manual, or automatic when the user turns it on, D7). There is no analytics, crash-reporting or licensing code. |
| 3. Data stays readable | No server response deletes or locks local data: `not_member` turns a vault into a read-only archive, account deletion leaves a local-only personal vault, and log out asks the user (D14, UI 12.2). The server has no remote-wipe operation. |
| 4. Open export | Export is part of `shellpp-core`, works offline and without an account. Formats: [storage-format.md](storage-format.md). |
| 5. Zero-knowledge sync | All key handling and encryption run on the client in `shellpp-core`. The server binary never links client cryptography: its only project dependency is `shellpp-sync-api`, which has none (the client crates are dev-dependencies of the server's end-to-end tests only, section 2.3). What the server sees is listed in [security.md](security.md). |

### 1.2 Constraints

- Stack (SPEC "Default stack"): Tauri 2 with a Rust core, React + TypeScript + Vite, xterm.js, russh + russh-sftp + ssh-key, SQLite, an axum server with PostgreSQL or SQLite.
- The Rust core owns all data, settings and secrets. Webviews render and collect input only (D23).
- Mobile reuse: `shellpp-core` and `shellpp-ssh` must build for iOS and Android. They do not depend on Tauri or on desktop-only crates, never look up OS directories themselves, and read user-chosen files through bytes and caller-provided resolvers, not through `std::fs` paths (Android pickers return content URIs) (stage 0 decision). Desktop-only features sit behind platform traits (section 5). russh uses the `ring` backend, partly because it builds more simply for mobile (D4).
- Security (SPEC "Security"): no secrets in logs, in plaintext on disk or in URLs; strict CSP; a minimal command set; auto-lock; clipboard clearing.
- No homemade cryptography: only standard constructions from the crates in section 8.2.
- Clean room: interfaces and formats come only from the specifications and the decisions.
- Licensing: `server/` is AGPL-3.0-only, everything else is Apache-2.0, and client code never depends on server code (section 2.3).
- Language: code, comments and documents in English; interface strings in the i18n catalog (D22).

### 1.3 Not in the MVP

Record signatures and rollback detection (D13, D14), biometrics (D5), FIDO2 beyond the stage 3 spike (D5), Mosh (D3), OSC 133 (D16), moving tabs between windows (D23), and (stage 0 decision) rotation of a user's X25519 key pair, shared known hosts in team vaults and server push for sync. The security-relevant ones are discussed in [security.md](security.md).

Custom CA certificates for sync servers are in the MVP now, together with pinning a server's certificate, for a server in a homelab: a CA the user adds or a certificate the user pins counts for that one server origin only ([sync-protocol.md](sync-protocol.md), section 2.1). [decision] (D93)

## 2. Components and repository layout

### 2.1 Layout

```text
.
├── apps/
│   └── desktop/              @shellpp/desktop: React + TypeScript frontend (stage 0)
│       ├── src/
│       │   ├── theme/        tokens.css, terminal/*.json, contrast-pairs.ts
│       │   ├── components/   <Name>/<Name>.tsx
│       │   ├── showcase/     component showcase page
│       │   ├── i18n/         en.json
│       │   ├── ipc/          the only module that calls commands and listens to events (stage 1)
│       │   └── stage.ts      stage flags (stage 1)
│       ├── e2e/              WebdriverIO specs (stage 1)
│       └── src-tauri/        crate shellpp-desktop (stage 1)
├── crates/
│   ├── core/                 shellpp-core
│   ├── ssh/                  shellpp-ssh
│   ├── term-local/           shellpp-term-local (desktop only)
│   └── sync-api/             shellpp-sync-api
├── server/                   shellpp-server, AGPL-3.0-only, its own Cargo workspace
├── tests/
│   ├── sshd/                 shellpp-test-sshd: russh SSH/SFTP test server
│   └── openssh/              OpenSSH Docker image for Linux integration tests
├── docs/                     specifications and design documents; Russian originals in docs/ru/
├── scripts/                  repository tooling
├── Cargo.toml                client Cargo workspace (stage 1)
├── deny.toml                 cargo-deny policy of the client workspace (stage 1)
├── package.json              pnpm workspace root
└── pnpm-workspace.yaml       packages: apps/*
```

The paths of tokens, terminal themes, components, the showcase and i18n come from D1, D22, D24 and D25. The `ipc/` module, `stage.ts`, `e2e/`, `tests/` and the separate server workspace are (stage 0 decision).

### 2.2 Components

| Path | Package | License | Platforms | Role | From stage |
|---|---|---|---|---|---|
| `apps/desktop` | `@shellpp/desktop` | Apache-2.0 | desktop; mobile later | React UI, xterm.js, i18n. No business logic and no persistent data. | 0 |
| `apps/desktop/src-tauri` | `shellpp-desktop`, binary `shellpp` | Apache-2.0 | Windows, macOS, Linux | Tauri host: windows, commands, events, platform trait implementations, deep links, updater | 1 |
| `crates/core` | `shellpp-core` | Apache-2.0 | all, including iOS and Android | data model, cryptography, storage, lock, in-memory index, keys, known hosts, import and export, history filter, notifications, sync client, platform traits | 1 |
| `crates/ssh` | `shellpp-ssh` | Apache-2.0 | all | SSH connections and authentication, channels, SFTP and the transfer queue, port forwarding, agent | 2 |
| `crates/term-local` | `shellpp-term-local` | Apache-2.0 | desktop only (D5) | local PTY, serial, in-house minimal Telnet | 2 (PTY), 9 (serial, Telnet) |
| `crates/sync-api` | `shellpp-sync-api` | Apache-2.0 | all, and the server | API request and response types, error codes, envelope header codec and size checks; no cryptography and no I/O | 1 (header codec), 7 (API types) |
| `server` | `shellpp-server` | AGPL-3.0-only | Linux container; any OS for development | sync server and admin CLI | 7 |
| `tests/sshd` | `shellpp-test-sshd` (`publish = false`) | Apache-2.0 | CI on all three OSes | russh SSH/SFTP/forwarding test server (D6) | 2 |
| `tests/openssh` | none | Apache-2.0 | Linux CI | OpenSSH server image (D6) | 2 |

Crate names come from section 7. Placing the envelope header codec in `shellpp-sync-api`, so that the server can validate envelopes without client cryptography, is a (stage 0 decision); the envelope itself is defined in [storage-format.md](storage-format.md).

### 2.3 Dependency direction and the license boundary

```text
apps/desktop (TypeScript) ── IPC only ──► shellpp-desktop

shellpp-desktop ─┬─► shellpp-ssh ──────────► shellpp-core ──► shellpp-sync-api
                 ├─► shellpp-term-local ───► shellpp-core
                 └─► shellpp-core

shellpp-server ──► shellpp-sync-api
shellpp-server (dev-dependencies only) ──► shellpp-core     sync end-to-end tests

shellpp-test-sshd ──► russh, russh-sftp                     no project crates
```

Rules:

- The arrows are the only allowed dependencies between project packages; there are no cycles.
- Nothing outside `server/` depends on `server/`, as a crate or by copying code. `server/` may depend on Apache-2.0 crates (CONTRIBUTING, "Licensing of contributions").
- Only `shellpp-desktop` depends on `tauri`. `shellpp-core` and `shellpp-ssh` never depend on `tauri`, `shellpp-term-local`, `keyring` or other desktop-only crates.
- `shellpp-desktop` links `shellpp-term-local` only for desktop targets (`[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`).
- The frontend reaches Rust only through `src/ipc/`. The pnpm workspace contains only `apps/*`.

Enforcement (stage 0 decision):

| Rule | Check |
|---|---|
| the server is outside the client build | `server/` has its own Cargo workspace and `Cargo.lock`; the root workspace sets `exclude = ["server"]` |
| no client dependency on the server | the client `deny.toml` bans the crate `shellpp-server`, and its license allow-list has no AGPL |
| `shellpp-core` and `shellpp-ssh` stay mobile-ready | from stage 1, CI runs `cargo check -p shellpp-core -p shellpp-ssh` for `aarch64-linux-android` (Linux job, with the runner's NDK) and `aarch64-apple-ios` (macOS job) |

| Artifact | Built from | License of project code | Distributed to |
|---|---|---|---|
| desktop packages | `apps/desktop`, `crates/core`, `crates/ssh`, `crates/term-local`, `crates/sync-api` | Apache-2.0 | users |
| server image | `server/`, `crates/sync-api` | AGPL-3.0-only (server), Apache-2.0 (`shellpp-sync-api`) | operators |

### 2.4 Crate internals

Module names are a (stage 0 decision). Each topic links to the document that defines it.

`shellpp-core`:

| Module | Responsibility | Defined in |
|---|---|---|
| `ids` | identifier constants (section 7) | this document |
| `crypto` | Argon2id, HKDF, XChaCha20-Poly1305, wrap blobs, HPKE wrapping, STREAM for recordings, zeroizing key types | [security.md](security.md), [storage-format.md](storage-format.md) |
| `store` | SQLite access, migrations with a backup first | [storage-format.md](storage-format.md) |
| `model` | record types, payloads, references, group inheritance, validation and the draft rule (D20, UI 1.3) | [data-model.md](data-model.md) |
| `vault` | vault keys, key epochs, wraps, member pins, rosters, epoch acceptance, re-encryption | [security.md](security.md), [sync-protocol.md](sync-protocol.md) |
| `lock` | first run, unlock, lock, auto-lock timer, password change, recovery | [security.md](security.md) |
| `index` | decrypted in-memory index, search and sorting (D10) | [data-model.md](data-model.md) |
| `keys` | key generation, import (OpenSSH, PEM, PuTTY `.ppk`), fingerprints, OpenSSH export (D5) | [data-model.md](data-model.md) |
| `known_hosts` | host key lookup, including hashed entries and revoked keys (D21) | [data-model.md](data-model.md) |
| `history` | capture rules and the secret detector (D16) | [security.md](security.md) |
| `import`, `export` | readers for ssh_config, PuTTY session values, CSV, JSON and known_hosts; writers for JSON, ssh_config, OpenSSH keys and known_hosts | [data-model.md](data-model.md), [storage-format.md](storage-format.md) |
| `settings` | per-device settings (`local_settings`) | [storage-format.md](storage-format.md) |
| `notify` | in-app notification center, memory only (D23, UI 1.5) | this document |
| `sync` | HTTP client, scheduler, merge, conflict copies, rotation jobs | [sync-protocol.md](sync-protocol.md) |
| `platform` | platform traits (section 5) | this document |

Other crates:

- `shellpp-ssh`: `client` (connect, jump hosts, proxy, authentication, keepalive, channels, reconnect policy), `agent` (system agent through `SSH_AUTH_SOCK`, the Windows OpenSSH agent pipe and Pageant; the in-app agent from stage 3; agent forwarding; D5), `sftp` (remote operations, the local file endpoint, the transfer queue), `forward` (local, remote and dynamic forwarding with a minimal SOCKS5 server).
- `shellpp-term-local`: `pty` (portable-pty; shell detection per UI 12.4), `serial` (serialport), `telnet` (RFC 854 with the ECHO, SGA, TTYPE and NAWS options).
- `shellpp-desktop`: `commands/<area>.rs`, `events.rs`, `windows.rs`, `platform/` (real implementations), `platform/mocks/` (only with the `e2e-mocks` feature), `deeplink.rs`, `updater.rs` (stage 9). It reads PuTTY sessions from the Windows registry under `cfg(windows)` and passes the values to `shellpp-core` (stage 0 decision).
- `shellpp-server`: axum routes, authentication, PostgreSQL and SQLite stores (sqlx), rate limiting, the purge job, optional SMTP, the admin CLI ([sync-protocol.md](sync-protocol.md)).

PEM and `.ppk` import use the RustCrypto primitives these formats specify (for example `aes`, `cbc`, `hmac`, `sha1`, `sha2`, `argon2`). This is parsing of documented formats, not new cryptography (D5).

### 2.5 Frontend

- One package, `@shellpp/desktop`. In stage 0 it is frontend-only, without `src-tauri` (D1).
- Each window keeps its own stores, built from a snapshot plus events (section 3.5). They hold view models, never key material.
- `src/ipc/` is the only module that calls `invoke`, `listen` or creates channels; the rest of the UI calls typed functions from it. Whether the TypeScript types are generated from Rust is decided in stage 1 (stage 0 decision).
- Terminal: xterm.js with the fit, search, webgl, web-links and unicode11 addons (SPEC). The UI takes history candidates from the xterm.js buffer (D16); `shellpp-core` applies the secret detector before anything is stored.
- Strings come from `src/i18n/en.json`; e2e tests find elements by `data-testid` (D22).
- Panels debounce edits by 500 ms before saving (stage 0 decision); validation and drafts follow UI 1.3.

## 3. Runtime model

### 3.1 Process and threads

| Part | Runs in | Notes |
|---|---|---|
| Tauri event loop, windows | main thread of the `shellpp` process | |
| Core services: SSH, SFTP, forwards, sync, timers | the Tokio multi-thread runtime that Tauri uses | one runtime for the whole app |
| Argon2id, bulk decryption, re-encryption, `SecretStore` calls | `spawn_blocking` threads | never on the event loop |
| SQLite | one connection owned by `core::store`; writes are serialized | reads come from the in-memory index while unlocked |
| Webviews | the OS webview (WebView2, WKWebView, WebKitGTK), one per window | UI only |

One process per user session opens the data directory (stage 0 decision). `tauri-plugin-single-instance` (with its deep-link support) passes the arguments of a second launch, such as a `shellpp://` link, to the running process, and the second process exits. Test instances with different `SHELLPP_DATA_DIR` values therefore run one at a time.

### 3.2 State ownership

| State | Owner | Lifetime | Webview access |
|---|---|---|---|
| master password, `master_key`, `kek` | `core::lock` | during unlock or password change only; zeroized right after use | the password is sent once as command input; nothing comes back |
| vault keys and derived keys | `core::vault` | while unlocked | never |
| decrypted records | `core::index` | while unlocked | view models through queries |
| secret fields (passwords, private keys, passphrases) | `core::index` | while unlocked | write-only; read only through an explicit reveal (UI 7.3) or copied to the clipboard by Rust |
| credentials kept for reconnects | `shellpp-ssh` | while unlocked | never |
| live SSH transports, forwards, transfers | `shellpp-ssh` | until closed; they survive lock (D9) | status only |
| access token | `core::sync` | memory only | never |
| refresh token | `core::store` | wrap blob on disk | never |
| settings | `core::settings` | persistent | read and write |
| notifications | `core::notify` | memory only (UI 1.5) | read |
| terminal scrollback | xterm.js in the webview | while the tab is open | it is the display |
| tabs, splits, panels, selection | the webview of each window | while the window is open | not applicable |

Installation identity (stage 0 decision): `db_id` and `device_id` are UUIDv7 values created at first run and stored in the database ([storage-format.md](storage-format.md)). They do not change on unlock, password change, recovery, sign-in to the account the device was last bound to, log out with the local copy kept, account deletion, migration or restore. A new pair exists only when a new database is created: a first run in an empty data directory, or after "Start with a new vault" (which sets the old database aside) and log out with "delete local copy" (which deletes it, D197). The one exception: binding to a different account (another server or another `user_id`) than the last bound one generates a new `device_id` first, and `db_id` stays ([sync-protocol.md](sync-protocol.md), section 4.7). They are kept separate so that such a reset of the sync identity (`device_id`) does not touch the keychain item named after `db_id`. A copied data directory is not a new device; [storage-format.md](storage-format.md) (section 12) describes what happens when a copy runs next to its original.

### 3.3 Lock states

| State | Meaning | Commands accepted |
|---|---|---|
| `first_run` | the database has no `local_auth` row | `app_info`, `state_snapshot`, `lock_status`, `settings_get`, `first_run_setup`, `window_open` |
| `locked` | keys and decrypted data wiped (D9, UI 13.3) | the same, without `first_run_setup`, plus `unlock`, `recovery_unlock` |
| `unlocked` | normal operation | all commands of the current stage |

- A guard in front of every command enforces the lock in Rust; the UI is not trusted to do it.
- Terminals keep running while locked (D9), so the output and size commands of running sessions (`session_attach`, `session_resize`, `session_ack`) are accepted in every state (stage 2); they reveal nothing and act only on sessions of the calling window.
- Auto-lock: one timer in `core::lock` for all windows (interval per UI 13.4, "Never" by default since D29). Only keyboard and mouse input in app windows counts (UI 13.3). Each window reports input with the `activity` command at most once every 30 seconds (stage 0 decision).
- "Remember on this device" unlocks with the device key at launch only; lock and auto-lock always ask for the password (UI 13.2).
- Lock rules, key wiping and the accepted risks are in [security.md](security.md).

### 3.4 Commands

Rules (stage 0 decision unless cited):

- Each command is declared in `src-tauri/src/commands/<area>.rs`, registered in a single `generate_handler!` list, and listed in the app manifest (`tauri_build::AppManifest::commands`), which gives every command its own `allow-<command>` permission. The capability file grants exactly that list.
- No generic commands: nothing that runs a shell command, reads an arbitrary file for the webview, makes an arbitrary HTTP request or passes calls through to a plugin. The Tauri `fs`, `shell` and `http` plugins are not used.
- The webview is treated as untrusted, because it renders remote data: terminal output, remote file names and records written by other vault members. Rust validates every argument.
- Secret arguments are converted to `secrecy` types on arrival; their `Debug` output is redacted and they are never logged.
- Errors have one shape. `code` is stable snake_case and the UI maps it to an i18n key:

```json
{"code": "locked", "details": {}}
```

- Local codes include `locked`, `invalid_password`, `not_found`, `validation_failed`, `cancelled`, `unavailable`, `io_error`, `ssh_error`, `port_in_use` and `host_key_revoked`. Sync errors pass through the codes of [sync-protocol.md](sync-protocol.md).

Command groups (names are indicative; each stage fixes them in code):

| Group | Commands | Stage |
|---|---|---|
| app | `app_info`, `state_snapshot`, `window_open`, `activity` | 1 |
| lock | `lock_status`, `first_run_setup`, `unlock`, `lock`, `recovery_unlock`, `password_change`, `recovery_key_show`, `recovery_key_reissue`, `remember_device_set` | 1 |
| records | `records_query`, `record_get`, `record_save`, `record_delete`, `record_move`, `record_copy`, `secret_reveal`, `secret_copy` | 1 |
| settings | `settings_get`, `settings_set` | 1 |
| notifications | `notifications_list`, `notifications_mark_read` | 1 |
| sessions | `session_open`, `session_attach`, `session_write`, `session_inject`, `session_resize`, `session_prompt_reply`, `session_cancel`, `session_close`, `session_reconnect`, `session_ack` | 2 |
| local terminal | `local_shells`, `local_open` (stage 2); `serial_ports`, `serial_open`, `telnet_open` (stage 9) | 2, 9 |
| clipboard | `clipboard_write_text`, `clipboard_read_text` | 2 |
| import and export | `import_pick`, `import_preview`, `import_apply`, `export_run` | 3 |
| files and transfers | `fs_list`, `fs_mkdir`, `fs_rename`, `fs_remove`, `fs_chmod`, `sftp_connect`, `sftp_disconnect`, `transfer_enqueue`, `transfer_control`, `transfer_conflict_reply`, `file_open` | 4 |
| forwards | `forward_start`, `forward_stop` | 5 |
| account and sync | `sync_now`, `conflict_resolve`, `account_*` | 7 |
| team vaults | `vault_*`, `member_*`, `invitation_*` | 8 |
| updates | `update_check`, `update_install` | 9 |
| e2e only | `e2e_mock`, compiled only with the `e2e-mocks` feature | 1 |

`fs_*` commands take an endpoint: `{"kind": "local"}` or `{"kind": "sftp", "connection_id": "…"}`. The local endpoint gives the UI access to the user's files by design (the SFTP local panel, UI 5.1), which is one reason for the webview hardening in section 3.7.

Plugins (stage 0 decision):

| Plugin | Called from | Permissions granted to the webview |
|---|---|---|
| core events | webview | `core:event:allow-listen`, `core:event:allow-unlisten` |
| core window | webview (custom title bar, UI 1.1) | only what the title bar needs: minimize, toggle maximize, is maximized, close, start dragging, set fullscreen |
| `tauri-plugin-single-instance` | Rust | none |
| `tauri-plugin-deep-link` | Rust | none |
| `tauri-plugin-dialog` | Rust (`FileDialogs`) | none |
| `tauri-plugin-clipboard-manager` | Rust (`Clipboard`) | none |
| `tauri-plugin-opener` | Rust (`ExternalLauncher`) | none |
| `tauri-plugin-updater` | Rust, stage 9 (D7) | none |

`core:default` is not granted.

### 3.5 Events and streams

Broadcast events go to every window (`emit`). Each payload carries `v`, a state version (u64) that increases with every broadcast event during the life of the process (stage 0 decision).

| Event | Payload besides `v` | Emitted when |
|---|---|---|
| `lock:changed` | `locked` | lock, unlock |
| `records:changed` | `vault_id`, `record_ids`, `classes` | local write, sync merge, import, re-encryption |
| `vaults:changed` | `vault_ids` | vault created or archived, role or epoch changed |
| `settings:changed` | `keys` | `settings_set` from any window |
| `sessions:changed` | `session_id`, `window`, `host_record_id`, `state`, `step` | connecting steps (UI 4.8), connected, ended, dropped, reconnecting |
| `forwards:changed` | `rule_id`, `state`, `error` | start, stop, error |
| `transfers:changed` | the changed items | coalesced to at most 4 per second |
| `sync:status` | `state`, `last_sync_at`, `error`, `conflicts` | start and end of a sync cycle |
| `notifications:changed` | `unread`, `latest` | new notification, list marked as read |
| `update:available` | `version` | stage 9 |

Window-targeted events go to one window (`emit_to`):

| Event | Payload | Target |
|---|---|---|
| `session:prompt` | `session_id` and the prompt: new host key, changed host key, username, password, passphrase, or one keyboard-interactive question with its echo flag | the window that owns the session |
| `transfers:conflict` | `transfer_id`, the name, and size and modification time of both files (UI 5.7) | the window that started the operation |
| `link:open` | the parsed `shellpp://` link | the focused or last focused window; held until unlock |

Terminal output uses a Tauri `Channel` per session subscriber: binary and ordered. It never goes through broadcast events. Output has flow control: the pane acknowledges the bytes its terminal has processed with `session_ack`; while more than 2 MiB sent to the pane are unacknowledged, the host stops reading the session's output (the SSH channel or the PTY then pushes back on the program that writes) and resumes below 512 KiB. Input waiting for the terminal is bounded as well: `session_write` and `session_inject` answer `unavailable` while more than 4 MiB wait, and the webview sends the input again later. `session_attach`, `session_resize` and `session_ack` are accepted while the app is locked, because running terminals keep running and showing output under the lock screen (D9).

Snapshot protocol (stage 0 decision): a new or reloaded window (1) subscribes to all broadcast events and buffers them, (2) calls `state_snapshot`, which returns `v` with the lock state, vaults, settings, notifications, forwards, transfers, sync status and the sessions of that window, (3) applies the buffered events whose `v` is greater. Records are loaded with `records_query`.

### 3.6 Windows

(D23, UI 1.4; details are a stage 0 decision)

- All windows are equal. Labels are `w-<n>` (`w-1` for the first window); no label is special, and the capability file targets `w-*`.
- A window opens from Window ▸ New window (UI 1.4), from a Connect option chosen with Alt (UI 2.4) or from "Duplicate in a new window" (UI 4.4).
- Each window has its own Vaults tab, SFTP tab and session tabs. A session belongs to the window that opened it, and tabs do not move between windows (D23).
- App-wide state: lock, records, settings, notifications, port forwards, the transfer queue, sync.
- Exit confirmation (UI 4.6) with several windows: closing a window that is not the last asks when that window has active sessions or transfers started from it; its sessions then end and those transfers are cancelled. Closing the last window, or Exit, asks when any session, forward or transfer is active.
- Closing the last window exits the app on every platform.
- The Settings window opens per app window; changes reach all windows through `settings:changed`.
- Lock applies to all windows at once (UI 13.3).

### 3.7 Webview hardening

Release CSP (`app.security.csp`, stage 0 decision; the normative string is in [security.md](security.md), section 13.1):

```text
default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'; frame-ancestors 'none'
```

- Tauri adds hashes or nonces for the bundled assets at build time. xterm.js inserts `<style>` elements at run time, so stage 2 added `'unsafe-inline'` to `style-src` only; the reason is recorded in [security.md](security.md), section 13.1. `script-src` is never relaxed. The development CSP (`app.security.devCsp`) may allow the Vite dev server.
- `app.withGlobalTauri = false`, the asset protocol is disabled, devtools exist only in debug builds. `Object.prototype` is frozen by the frontend entry once the xterm.js engine has loaded and before the backend starts; `app.security.freezePrototype` is `false` because Tauri's earlier freeze breaks xterm.js 6 (reason in [security.md](security.md), section 13.1).
- Windows load only bundled assets. No remote URL is ever loaded in a webview, and capabilities have no `remote` entries.
- Links (terminal web links, Help, the links in Settings 12.1) open in the system browser through `ExternalLauncher`, only for `http` and `https`.
- Untrusted text is rendered as React text; `dangerouslySetInnerHTML` is forbidden by a lint rule. Clipboard access from terminal escape sequences (OSC 52) is not enabled.

### 3.8 Diagnostic logs

(stage 0 decision)

- All crates log through `tracing`. The desktop app writes `shellpp.log` in the logs directory (section 5.3), rotated at 5 MiB, 5 files kept.
- Logs contain event names, ids, states and error codes. They never contain secrets, host addresses, usernames, command text, terminal data or remote file names.
- Webview console output is not written to the log in release builds.

## 4. Main flows

Actors: UI (the webview of window W), CMD (commands and events in `shellpp-desktop`), CORE (`shellpp-core`), SSH (`shellpp-ssh`), OS. The flows below fix which component does what; protocol and key details are in the linked documents. First run, registration, sign-in, password change, recovery, invitations, member removal, log out and account deletion are described in [security.md](security.md) and [sync-protocol.md](sync-protocol.md).

### 4.1 Start, unlock and lock

Start:

1. OS starts `shellpp`. If an instance is already running, the arguments go to it and this process exits.
2. CMD resolves the directories (section 5.3), starts logging, empties `tmp/`, builds the platform implementations (section 5.1) and starts CORE.
3. CORE opens `shellpp.db` and runs pending migrations, which take a backup first ([storage-format.md](storage-format.md), section 11). Without a `local_auth` row the state is `first_run`, otherwise `locked`.
4. CMD opens window `w-1`. UI calls `lock_status` and shows the first-run screen (UI 13.1) or the unlock screen (UI 13.2).
5. If "remember on this device" is on, CORE tries the device key at this point instead of waiting for the password (steps 2–3 of Unlock are replaced; on any failure the password is asked for).

Unlock:

1. UI sends `unlock` with the password and clears the field.
2. CORE, on a blocking thread: reads `local_auth`, checks the KDF parameters against the client floor and the platform ceiling, runs Argon2id with the salt bound to `local_auth.kdf_email` ([security.md](security.md), section 5.4), derives `kek`, wipes `master_key`.
3. CORE opens the wrap of the current personal vault key with `kek` and wipes `kek`. A failed unwrap is `invalid_password`; nothing else is touched.
4. CORE opens older epochs through the epoch chain, decrypts `user_keypair` if present, and opens the HPKE wraps of team vaults after checking the sender's pin.
5. CORE decrypts records into the in-memory index. The `history` and `connection_log` classes are decrypted on first use (D10).
6. CORE enters `unlocked`, starts the auto-lock timer and releases deferred work in this order: waiting reconnects, forwards set to start on application launch, the sync cycle (when an account is bound).
7. CMD emits `lock:changed`; every window takes a snapshot and returns to its previous state (UI 13.3).

Lock:

1. Trigger: the Lock menu item (`lock`) or the auto-lock timer.
2. CORE stops the sync scheduler (an interrupted cycle leaves no partial state, because every pulled page is one transaction) and parks reconnect and autostart timers until unlock.
3. CORE wipes vault keys, derived keys, the index, credentials kept for reconnects and the access token. Live SSH transports, forwards and transfers keep running (D9).
4. CMD emits `lock:changed`. Every window drops its stores and shows the unlock screen over its content; terminals keep receiving output underneath.
5. No new connection starts while locked. Agent requests that need wiped keys fail until unlock ([security.md](security.md)).

### 4.2 Connect with TOFU

1. UI sends `session_open` with the host record id (or a quick-connect string, UI 2.1) and its window label, receives a `session_id`, and calls `session_attach` with a channel for the output. The tab shows the connection screen (UI 4.8).
2. CORE resolves the effective configuration: host fields, group inheritance ([data-model.md](data-model.md)), global defaults (keepalive 30 s, TERM `xterm-256color`), and credentials: an identity or key in the same vault, or for a host in a team vault the member's `credential_binding` (D13).
3. SSH connects through the jump hosts (stage 5), each with its own host key check, and the proxy if any (stage 5), then the target. Every step is reported with `sessions:changed`.
4. Host key check: SSH asks CORE to look up (host, port, key) in the app's own known hosts store only (D21). Hashed entries are matched with HMAC.
   - Match: continue.
   - Unknown: the handshake waits; CMD sends `session:prompt` (address, key type, SHA256 fingerprint) to W. "Accept and save" writes a `known_host` record to the personal vault and continues; "Connect once" continues without saving; "Cancel" disconnects (UI 10.2).
   - Changed: the connection is blocked and W gets `session:prompt` with the old and new fingerprints. Only "Replace saved key" updates the record and continues (UI 10.2).
   - Revoked (`@revoked`, D21): the connection is blocked with `host_key_revoked` and W shows the danger-colored warning without a button to continue (UI 10.2).
   - If the server closes the connection while the user decides, SSH reconnects once after an accept or replace (stage 0 decision).
5. Authentication: SSH tries the methods the server allows with what the configuration provides (public key from a key record or an agent, password, keyboard-interactive). A missing username, password or passphrase, and each keyboard-interactive question, becomes a `session:prompt`; the answer comes back with `session_prompt_reply` (UI 4.8). With "Save" checked, CORE stores the value in the identity or key record.
6. Session setup: PTY request with TERM and size, environment variables, agent forwarding if enabled (D5), shell; the startup snippet (stage 5); OS detection in a separate exec channel when the server did not report the OS (UI 12.4).
7. Running: output bytes go through the channel to xterm.js; typed input and size go through `session_write` and `session_resize`. Input the app sends on its own (a snippet insert or run, UI 4.2 and 9.3, and a run on several hosts, UI 9.4) goes through `session_inject`, never `session_write`, and the core sends the startup snippet through the same injected path, so CORE knows which bytes were injected. OS detection never writes to the session (step 6). History candidates go to CORE, which applies the host's history setting, drops any line that contains injected input, and applies the secret detector before writing a `history_entry` (D16, [security.md](security.md), section 10.1). The connection log entry is written at start and end (stage 6).
8. End (UI 4.6, D18): exit status 0 closes the tab; a non-zero status shows "Session ended with code N" without reconnecting; a transport error or keepalive timeout marks the session dropped and, with Autoreconnect on, starts attempts after 1, 2, 4, 8, 16 and 32 seconds, then every 60 seconds. While locked, attempts wait for unlock.

### 4.3 SFTP transfer queue

One `TransferQueue` in `shellpp-ssh` serves all windows (stage 0 decision). It runs at most N transfers at once (UI 12.6, 3 by default).

1. UI sends `transfer_enqueue` (drag and drop, or Copy to target directory) with a source endpoint and paths and a target endpoint and folder. An endpoint is the local file system or an SFTP connection.
2. The queue expands folders into items and marks them queued; UI 5.6 shows one row per transfer.
3. When a slot is free, the next item starts. If the target name exists, the policy from UI 12.6 applies. With "Ask" (the default), W gets `transfers:conflict` and answers Replace, Skip or Rename with "Apply to all" for the rest of the same operation (UI 5.7). Rename picks a free name such as `report (1).txt`.
4. Data is written to a temporary name next to the target, `.<name>.shellpp-part`, and renamed to the final name on success; Replace removes the old file only at that moment (stage 0 decision). Remote-to-remote transfers stream through memory without a local file.
5. Progress (bytes, total, speed) is reported with `transfers:changed`, coalesced to at most 4 events per second.
6. `transfer_control`: Pause stops after the current chunk and keeps the partial file; Resume continues from its size; Cancel deletes the partial file; Retry restarts a failed item. Finished items stay listed until "Clear finished" (UI 5.6).
7. Transfers keep running while locked (UI 13.3). Closing a window cancels the transfers that use its connections, after the confirmation in section 3.6.

Open and Open with (UI 5.4): the file is downloaded through the queue into `tmp/` ([storage-format.md](storage-format.md), section 9) after the warning; `ExternalLauncher` opens it with the program from "Program for Open", the system association, or the program the user picks; a file watcher queues an upload after each change and adds a notification (UI 1.5). Following [storage-format.md](storage-format.md), "closed" means the SFTP tab or connection that opened the file closes; `tmp/` is also emptied on exit and on start.

### 4.4 Port forwarding

One `ForwardManager` in `shellpp-ssh` runs all rules (stage 0 decision). A rule is a `forwarding_rule` record; its runtime state (stopped, starting, running, error) lives in memory and is broadcast with `forwards:changed`.

Starts: `forward_start` (the Start item or a double click, UI 8.2), "start automatically when connecting to the host", and "start on application launch" (after the first unlock, UI 8.4).

Connections (stage 0 decision):

- A rule started together with a session uses that session's SSH connection and stops when the session ends.
- A rule started manually or at launch uses a connection owned by the manager, shared by all such rules for the same host. It reconnects with the delays of section 4.2 and waits for unlock while locked.

| Type | Start | Each incoming connection |
|---|---|---|
| Local | bind a TCP listener on the bind address and port (127.0.0.1 by default; 0.0.0.0 only by explicit choice, UI 8.4). A busy port fails with `port_in_use` and the port number. | open a `direct-tcpip` channel to the destination and copy both ways |
| Remote | send a `tcpip-forward` request for the remote address and port (127.0.0.1 by default, UI 8.3) | accept the `forwarded-tcpip` channel, connect to the local target, copy both ways |
| Dynamic | bind a local SOCKS5 listener (127.0.0.1:1080 by default). The in-house SOCKS5 server supports no authentication and the CONNECT command for IPv4, IPv6 and domain names (stage 0 decision). | open a `direct-tcpip` channel to the requested address |

Stop closes the listener (Local, Dynamic) or sends `cancel-tcpip-forward` (Remote), then closes open channels. Errors set the `error` state with a code. Running forwards keep running while locked (UI 13.3).

### 4.5 Sync cycle

The `SyncScheduler` in `core::sync` runs only while unlocked with a bound account. Requests, statuses and merge rules are in [sync-protocol.md](sync-protocol.md).

Triggers: unlock (which includes app start), 2 seconds after the last local change, every 60 seconds, and "Sync now". Only one cycle runs at a time; a trigger during a cycle schedules one more cycle.

1. Token: CORE uses the in-memory access token or refreshes it with the refresh token, unwrapped with `local_secrets_key`. A refused refresh asks the user to sign in again; local data is untouched.
2. `GET /v1/vaults` detects new vaults, role and epoch changes, lost memberships and `rotation_pending`. A new epoch is used only after the member list has been fetched and the epoch accepted, after the personal vault pull of step 3 ([security.md](security.md), section 5.7).
3. Vaults are processed personal vault first, because it holds `user_keypair` and the member pins needed for team vault keys (stage 0 decision). For each vault:
   1. pull pages for the enabled classes until `has_more` is false. Each page is decrypted, checked, merged by HLC (conflict copies included) and stored together with the cursor in one SQLite transaction; the index is updated and `records:changed` emitted;
   2. `cursor_expired` starts a full resync of that vault, which keeps and pushes back records missing from the snapshot; `not_member` turns it into a read-only archive and deletes nothing;
   3. push pending records of the enabled classes in batches within the limits; per-item conflicts are merged and pushed again, and `stale_key_epoch` refreshes the keys, re-encrypts and retries.
4. Background jobs between cycles: re-encryption after a key rotation and, on owner devices, rotation of vaults with `rotation_pending`. That rotation runs on its own only when every remaining member is already an `accepted` recipient in the client's roster, to whom this account wrapped the current epoch (the entry's `epoch_commit` matches the key this device holds) and whose pin is verified; otherwise it waits for the owner's confirmation, so the server cannot add a recipient through `rotation_pending`, a withheld pin or a roster revision written on a forked view ([security.md](security.md), section 5.7).
5. `sync:status` reports the time, an error code if any and the number of unresolved conflicts (UI 12.2, 13.6). Network errors only mark the status; local changes stay pending until the next cycle.

## 5. Platform abstraction

### 5.1 Traits

All traits live in `shellpp_core::platform`. At startup `shellpp-desktop` builds one `Platform` value with an `Arc<dyn …>` per trait and hands it to the services (stage 0 decision). A feature whose trait has no implementation on a platform is hidden there.

| Trait | Used for | Desktop implementation | `e2e-mocks` build | Mobile (later) |
|---|---|---|---|---|
| `SecretStore` | the device key of "remember on this device" (D8) | `keyring`, in `shellpp-desktop` (D5) | in-memory map | iOS keychain; Android needs its own implementation (D5) |
| `PtyBackend` | local terminal | `shellpp-term-local` (portable-pty) | real, with the CI runner's shell | none; the item is hidden |
| `SerialBackend` | serial connections | `shellpp-term-local` (serialport) | virtual loopback port | none; the item is hidden |
| `FileDialogs` | open and save pickers (import, export, key files, Open with) | `tauri-plugin-dialog` | answers queued by the test | platform pickers |
| `Clipboard` | copy and paste; secrets copied by Rust are cleared after 30 seconds if the clipboard still holds them (UI 7.3) | `tauri-plugin-clipboard-manager` | in-memory, readable by the test | plugin on mobile |
| `ExternalLauncher` | open a file with the configured, default or chosen program; open `http` and `https` links | `tauri-plugin-opener` | records requests, launches nothing | platform open and share |
| `NotificationSink` | delivery of in-app notifications; no system notifications (UI 1.5) | `notifications:changed` events | same as desktop | same |

- The system SSH agent (Unix socket, the Windows OpenSSH agent pipe, Pageant) is OS-specific code inside `shellpp-ssh` behind `cfg`, not a platform trait.
- Biometric unlock would be a second way to release the device key and is deferred (D5).

### 5.2 `SecretStore`

```rust
pub enum Availability {
    Available,
    Unavailable(UnavailableReason), // NoSecretService, UnsignedBuild, Unsupported
}

pub trait SecretStore: Send + Sync {
    fn availability(&self) -> Availability;
    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError>;
    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError>;
    fn delete(&self, account: &str) -> Result<(), SecretStoreError>;
}
```

- The API is blocking and is called from `spawn_blocking` (stage 0 decision).
- Service `shellpp`, account `device-key:<db_id>` (section 7). It is the only item the app stores, and only while "remember on this device" is on (D8). Turning the option off deletes the item (stage 0 decision). The stored encoding is in [storage-format.md](storage-format.md).
- `Unavailable` covers Linux without Secret Service, macOS builds without a Developer ID signature (D7; a compile-time constant set by the release workflow, stage 0 decision) and Android until an implementation exists. The option is then hidden (UI 13.4); there is no fallback to a file (D5).
- `keyring` features: `apple-native`, `windows-native`, `async-secret-service`, `crypto-rust` and `tokio`; never `sync-secret-service` or `vendored`, which use libdbus (D4).

### 5.3 Directories

All directory names use `shellpp` (D2), not the bundle identifier. Unix permissions: directories `0700`, files `0600` (stage 0 decision).

| OS | Data (database, recordings, backups) | Diagnostic logs | Temporary files (SFTP Open) |
|---|---|---|---|
| Windows | `%LOCALAPPDATA%\shellpp\` | `%LOCALAPPDATA%\shellpp\logs\` | `%LOCALAPPDATA%\shellpp\tmp\` |
| macOS | `~/Library/Application Support/shellpp/` | `~/Library/Logs/shellpp/` | `~/Library/Caches/shellpp/tmp/` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/shellpp/` | `${XDG_STATE_HOME:-~/.local/state}/shellpp/logs/` | `${XDG_CACHE_HOME:-~/.cache}/shellpp/tmp/` |
| iOS, Android (later) | app sandbox, `shellpp/` under the app's support or files directory | same sandbox, `logs/` | same sandbox, `tmp/` |

- Windows uses the local, non-roaming profile on purpose: the database belongs to one device.
- `SHELLPP_DATA_DIR` replaces the data directory for tests and portable use. When it is set, logs go to `<dir>/logs/` and temporary files to `<dir>/tmp/`, so that test runs never share or wipe each other's directories (stage 0 decision).
- `shellpp-desktop` resolves the directories and passes them to `shellpp-core` in its configuration; the core never looks them up itself (mobile reuse).
- Files inside the data directory are listed in [storage-format.md](storage-format.md).

### 5.4 Mocks for e2e builds

- Cargo feature `e2e-mocks` on `shellpp-desktop`, off by default (D6; the name is a stage 0 decision).
- With the feature, `platform::install()` takes the implementations from `platform/mocks/` (table in section 5.1). It also registers the `e2e_mock` command and adds `capabilities/e2e.json`, which grants only that command.
- `e2e_mock` lets a test queue file dialog answers, read and set the mock clipboard, list launch requests, inject a `shellpp://` link and drive the mock serial port.
- The frontend's e2e build mode exposes a `window.__SHELLPP_E2E__` helper that calls `e2e_mock`; production builds do not contain it.
- Release safety: `#[cfg(all(feature = "e2e-mocks", not(debug_assertions)))]` triggers `compile_error!`, so a release-profile build cannot contain mocks, and the release workflow never enables the feature.
- D6 requires mocks for native dialogs, the clipboard and launching external programs. The `SecretStore`, serial port and deep-link mocks are a (stage 0 decision): WebDriver cannot see the OS keychain, CI runners have no serial ports, and test builds do not register the URL scheme. The updater is inactive in e2e builds.

## 6. Stages and stage flags

### 6.1 Stage map

| Stage | Scope | Source |
|---|---|---|
| 0 | the five design documents; `apps/desktop` as a frontend-only package with tokens, terminal themes, base components and a showcase page; contrast and render tests, `tsc` and eslint in one Linux CI job | SPEC, D1 |
| 1 | Cargo workspace, Tauri app, main window shell (UI 1), encrypted storage, master password with UI 13.1–13.4 (the Settings window opens with only the Security section; other sections stay hidden until their stage, stage 0 decision), Hosts screen, host panel and group panel (UI 2–3) | SPEC, D9, D20 |
| 2 | SSH terminal with password and key authentication and the short "+ → Key" path, known hosts and TOFU, tabs, splits, themes, terminal sidebar, new tab page (UI 4, 6, 10); local terminal; system ssh-agent and Pageant; agent forwarding; OS detection; autoreconnect, keepalive and TERM stored without a settings screen | SPEC, D3 |
| 3 | Keychain (UI 7), the import dialog with all formats (UI 2.5), the built-in agent, export to JSON and ssh config; the FIDO2 spike comes before this stage | SPEC, D3, D5 |
| 4 | SFTP (UI 5) with the transfer queue and the name conflict dialog | SPEC, D19 |
| 5 | port forwarding, jump hosts, proxy, snippets (UI 8, 9) | SPEC |
| 6 | logs, settings, shortcuts, keyword highlighting (UI 11, 12); the biometrics decision follows this stage | SPEC, D3, D5 |
| 7 | sync server and client, multiple devices, the connect wizard and conflicts (UI 12.2, 13.5, 13.6) | SPEC |
| 8 | team vaults, roles, invitations, sharing through vaults (UI 2.6, 12.3) | SPEC, D13, D14 |
| 9 | workspaces, broadcast, Telnet and serial connections, auto-update and packaging (UI 12.7) | SPEC, D3, D7 |
| 10 | later stages: mobile apps, multiplayer, cloud import, autocomplete, LLM snippet generation; Mosh as its own stage | SPEC, D3 |

### 6.2 Stage flags

- Elements of future stages are hidden, not disabled. Their e2e tests are written in the stage that implements them, and every stage report lists what is still hidden (D3).
- `apps/desktop/src/stage.ts` holds `BUILD_STAGE` and a map from feature id to the stage that implements it; `isVisible(feature)` compares the two. Hidden elements are not rendered, so their `data-testid` values are absent from the DOM (stage 0 decision).
- The flag is a build constant: not a setting, not an environment variable and never controlled remotely (principle 2).
- Rust has no stage flags: a command exists from the stage that implements it.
- A script prints the features whose stage is greater than `BUILD_STAGE`; the stage report includes its output (stage 0 decision).

Hidden elements known in stage 0 (by UI section):

| Element | Visible from stage |
|---|---|
| Known Hosts section (1.2) | 2 |
| New Local Terminal and the Terminal button (1.4, 2.2), Ctrl+L | 2 |
| Keychain section (1.2); Import (2.2, 2.3, 10.1); the Account & Sync settings section with data export only (12.1, 12.2) | 3 |
| SFTP tab (1.1, 1.4), Connect via SFTP (2.4) | 4 |
| Port Forwarding and Snippets sections (1.2), Ctrl+P; Startup snippet, Host Chaining, Proxy (3.1); Snippets tab of the terminal sidebar (4.2) | 5 |
| Logs section (1.2); the Settings sections Terminal, SFTP and Logs, Shortcuts and the Theme dropdown (12.1, 12.4–12.6) | 6 |
| Connect sync server and the Synchronization block (12.2, 13.5); conflict badges and dialog (13.6) | 7 |
| vault switcher and member avatars (1.1, 2.2); Collaborate, Share this host, Share this snippet; Move to and Copy to another vault (2.4, 2.6, 3.1, 8.2, 9.3); SSH ID and Team (12.3) | 8 |
| update button (1.1), Check for updates (1.4), Updates (12.7); Add to Workspace (2.4), Create a workspace (6); Broadcast input (4.7); New Serial Connection and the Serial button (1.4, 2.2), Ctrl+Alt+S; Add Telnet (3.1, 3.3) | 9 |
| Autocomplete (4.2, 12.4), Start multiplayer (4.4), script generation (9.5), cloud integrations (2.2), Mosh (3.1) | 10 or later |
| biometric unlock (13.4) | after the separate decision that follows stage 6 (D5) |

## 7. Identifiers

The display name is "Shell++"; every technical identifier uses `shellpp` (D2). Rust code reads them from `shellpp_core::ids`, and from stage 1 CI checks that `tauri.conf.json` matches these constants (stage 0 decision). The name of the local working-copy directory never appears in code, identifiers or UI (D2).

| Item | Value | Source |
|---|---|---|
| Display name | `Shell++` | D2 |
| Technical name, slug | `shellpp` | D2 |
| Repository | `https://github.com/proxyboy228/shellpp` | owner |
| Bundle identifier, Tauri `identifier`, app id | `io.github.proxyboy228.shellpp` | D2 |
| Desktop binary name | `shellpp` | stage 0 decision |
| npm package | `@shellpp/desktop` (`apps/desktop`) | D2 |
| Crate `crates/core` | `shellpp-core` | D2 |
| Crate `crates/ssh` | `shellpp-ssh` | D2 |
| Crate `crates/term-local` | `shellpp-term-local` | D5; name stage 0 decision |
| Crate `crates/sync-api` | `shellpp-sync-api` | stage 0 decision |
| Tauri crate | `shellpp-desktop` (`apps/desktop/src-tauri`) | stage 0 decision |
| Test server crate | `shellpp-test-sshd` (`tests/sshd`, not published) | stage 0 decision |
| Server crate and binary | `shellpp-server`; admin CLI `shellpp-server admin …` | stage 0 decision |
| Docker image | `ghcr.io/proxyboy228/shellpp-server` | stage 0 decision |
| URL scheme | `shellpp://` | D2 |
| Host link | `shellpp://host/<record_id>` (UUID text) | UI 2.4 |
| Invitation link | `shellpp://invite/<invitation_token>?server=<percent-encoded https origin>` | D14; layout stage 0 decision |
| OS keychain service | `shellpp` | D2 |
| OS keychain account | `device-key:<db_id>` | D8; name stage 0 decision |
| Export format ids | `shellpp.export.v1` (plain JSON), `shellpp.export.encrypted.v1` (encrypted JSON) | D2; encrypted id stage 0 decision |
| Terminal theme schema id | `shellpp.terminal-theme.v1` (files carry `"version": 1`) | D25; id stage 0 decision |
| JSON Schema `$id` form | `urn:shellpp:schema:<name>:v<N>`, for example `urn:shellpp:schema:export:v1` | stage 0 decision |
| Server environment prefix | `SHELLPP_` | stage 0 decision |
| HTTP API base path | `/v1` | stage 0 decision |
| HKDF, hash and AAD label prefix | `shellpp/v1/` (client, including the `shellpp/v1/kdf-salt` label of the Argon2id salt), `shellpp-server/v1/` (server-only keys) | stage 0 decision |
| Token prefixes | access `shellpp_at_`, refresh `shellpp_rt_`, login ticket `shellpp_lt_`, recovery ticket `shellpp_rc_` | stage 0 decision |
| User-Agent | sync API `shellpp/<semver>`; update manifest request `shellpp/<semver>` (no identifiers, D7, D97) | stage 0 decision, D97 |
| Default update manifest URL | `https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json` | D7; URL stage 0 decision |
| Cargo feature for e2e mocks | `e2e-mocks` | stage 0 decision |
| Client environment variables | `SHELLPP_DATA_DIR` (data directory), `SHELLPP_TEST_SSHD` (tests only: `openssh` or `russh`) | stage 0 decision |

Frontend paths (D1, D22, D24, D25; the rest stage 0 decision):

- tokens: `apps/desktop/src/theme/tokens.css`;
- terminal themes: `apps/desktop/src/theme/terminal/graphite.json`, `paper.json`;
- contrast pairs: `apps/desktop/src/theme/contrast-pairs.ts`, a TypeScript module that mirrors [design-system.md](design-system.md), section 7.3;
- components: `apps/desktop/src/components/<Name>/<Name>.tsx`;
- showcase page: `apps/desktop/src/showcase/`;
- i18n catalog: `apps/desktop/src/i18n/en.json`, with dotted keys such as `hosts.search.placeholder`;
- `data-testid`: kebab-case `<area>-<element>[-<qualifier>]`, for example `hosts-search-input`; components accept a `testId` prop.

Frozen once users have data: the bundle identifier, keychain service and account, directory names, URL scheme and link layouts, export and schema ids, HKDF, hash and AAD labels and token prefixes. Changing any of them needs a migration plan. Command names, event names and window labels are internal and may change between versions.

## 8. Dependency policy

### 8.1 License lists

| List | Licenses (SPDX) | Applies to |
|---|---|---|
| Shipped (D4) | MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, MPL-2.0, Zlib, BSL-1.0, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, CDLA-Permissive-2.0, public domain (SQLite); OFL-1.1 for fonts only | Rust crates of the client and the server, npm `dependencies`, fonts, icons, bundled terminal color schemes |
| Development only (D4) | the shipped list plus BlueOak-1.0.0 and CC-BY-4.0 | npm `devDependencies` and other build, lint and test tools that are never shipped |
| Platform dependencies (D4) | WebView2, WebKitGTK, GTK, glibc | not shipped; LGPL libraries bundled inside an AppImage are listed in `THIRD_PARTY_NOTICES.md` |
| Forbidden | GPL and AGPL in anything shipped with the client; source-available licenses (for example BUSL-1.1, SSPL); missing or unclear licenses | everything; the AGPL of `shellpp-server`'s own code is the only AGPL in the repository |

- SPDX `OR` needs one allowed choice; `AND` needs all parts allowed.
- `libsqlite3-sys` declares MIT but bundles SQLite, which is public domain; `THIRD_PARTY_NOTICES.md` lists SQLite separately.
- A license outside these lists needs an owner decision before the dependency is added. Older crate versions under Unicode-DFS-2016 are replaced by versions under Unicode-3.0 rather than extending the list (stage 0 decision).
- If a transitive development dependency moves to a forbidden license, the last allowed version is pinned with pnpm `overrides` and the upstream change is tracked (stage 0 decision).

### 8.2 Main dependencies

Licenses below are the expected SPDX values; cargo-deny and the npm license check are authoritative, and `THIRD_PARTY_NOTICES.md` records exact versions when a dependency is added.

| Area | Dependencies | Expected license | Where |
|---|---|---|---|
| Desktop shell | `tauri` 2 and the plugins of section 3.4 | MIT OR Apache-2.0 | `shellpp-desktop` |
| SSH | `russh` (default features off, `ring` backend), `russh-sftp`, `ssh-key` | Apache-2.0; Apache-2.0; Apache-2.0 OR MIT | `shellpp-ssh`, `shellpp-core` (`ssh-key`) |
| Crypto | `argon2`, `hkdf`, `sha2`, `hmac`, `chacha20poly1305` (with the `aead` stream module), `hpke`, `rand_core`, `getrandom`, `zeroize`, `secrecy` | MIT OR Apache-2.0 | `shellpp-core` |
| Crypto | `x25519-dalek`, `subtle` | BSD-3-Clause | `shellpp-core` |
| Key formats | RustCrypto `pkcs8`, `pkcs1`, `sec1`, `p256`, `p384`, `aes`, `cbc`, `sha1` (PuTTY `.ppk` and HMAC-SHA1 matching of hashed `known_hosts` entries, D21), `rsa` (D4 exception); `ed25519-dalek` | MIT OR Apache-2.0; BSD-3-Clause | `shellpp-core` |
| Storage, ids | `rusqlite` (`bundled`), `uuid` (`v7`), `serde`, `serde_json`, `data-encoding` | MIT; Apache-2.0 OR MIT; MIT OR Apache-2.0; MIT OR Apache-2.0; MIT | `shellpp-core` |
| HTTP | `reqwest` with `rustls` (ring provider, system roots plus, for one sync origin, a pinned certificate or an added CA: [sync-protocol.md](sync-protocol.md), section 2.1) | MIT OR Apache-2.0; Apache-2.0 OR ISC OR MIT | `shellpp-core` |
| Runtime, logs | `tokio`, `tracing` | MIT | all crates |
| Keychain | `keyring` (features in section 5.2) | MIT OR Apache-2.0 | `shellpp-desktop` |
| Local terminal | `portable-pty`, `serialport` | MIT; MPL-2.0 | `shellpp-term-local` |
| Server | `axum`, `rusqlite` and `postgres` (D101), `argon2`, `totp-rs`, `governor`, optional `lettre` | MIT; MIT, MIT OR Apache-2.0; MIT OR Apache-2.0; MIT; MIT; MIT | `shellpp-server` |
| Frontend | `react`, `react-dom`, `@xterm/xterm` and addons, `@tauri-apps/api`, `lucide-react` | MIT; MIT; MIT; MIT OR Apache-2.0; ISC | `@shellpp/desktop` |
| Fonts | Inter, JetBrains Mono | OFL-1.1 | `@shellpp/desktop` |
| Dev tools | Vite, Vitest, Testing Library, jsdom, TypeScript, eslint, WebdriverIO, tauri-driver | MIT, Apache-2.0, BlueOak-1.0.0 (dev list) | not shipped |

Crates for the file watcher (stage 4), the Windows registry (stage 3) and log rotation (stage 1) are chosen in their stages under the same rules. New cryptographic crates need an explicit review against "vetted crates only" (SPEC); `hpke` is pinned to an exact version after an audit check when it is added (stage 8).

### 8.3 cargo-deny rules

Two configurations: `deny.toml` for the client workspace and `server/deny.toml` for the server workspace, with the same rules except where noted (stage 0 decision for the layout).

| Check | Rule | Source |
|---|---|---|
| licenses | allow the shipped list; Rust dev-only crates that need the development list get a per-crate exception with a reason | D4 |
| licenses | `server/deny.toml` allows `AGPL-3.0-only` only for the crate `shellpp-server` | stage 0 decision |
| bans | deny `aws-lc-fips-sys`, and the feature `fips` of `aws-lc-rs` and `rustls` | D4 |
| bans | deny `ring` versions `< 0.17.10` and `aws-lc-sys` versions `< 0.39` | D4 |
| bans | deny the crates `libdbus-sys` and `dbus`, and the `keyring` features `sync-secret-service` and `vendored` | D4 |
| bans | deny `openssl-sys` and `openssl`: TLS is rustls and cryptography comes only from the crates in section 8.2 | stage 0 decision |
| bans | client only: deny `shellpp-server` | stage 0 decision |
| advisories | deny known vulnerabilities, except `RUSTSEC-2023-0071` (`rsa`, timing side channel) with the reason "signing happens locally; risk is low; upstream is tracked" | D4 |
| sources | crates.io only; a git dependency needs an entry with a reason | stage 0 decision |

npm: `pnpm licenses:check` (root `package.json`) checks production dependencies against the shipped list and development dependencies against the development list; D4 names license-checker for this job.

### 8.4 Adding a dependency

1. Check the license against section 8.1 and prefer pure-Rust crates that build for iOS and Android in `shellpp-core` and `shellpp-ssh`.
2. Record it in `THIRD_PARTY_NOTICES.md` in the same pull request (SPEC "Clean room").
3. CI runs cargo-deny for both workspaces and the npm license check; a failure blocks the merge.

## 9. Testing

### 9.1 Test layers

| Layer | Tool | Location | Runs on | From stage |
|---|---|---|---|---|
| contrast test (D24) | Vitest | `apps/desktop` | Linux | 0 |
| component render tests | Vitest, Testing Library, jsdom | `apps/desktop/src` | Linux | 0 |
| types and lint | `tsc`, eslint; `rustfmt`, `clippy -D warnings` from stage 1 | all packages | Linux | 0 |
| Rust unit tests: encryption, merge, ssh_config parser, history secret filter (SPEC "Quality") | `cargo test` | `crates/*` | Linux, Windows, macOS | 1 |
| known-answer tests for envelopes, wraps, KDF and HKDF labels, using the vectors of [storage-format.md](storage-format.md) and [security.md](security.md) | `cargo test` | `crates/core/tests` | Linux, Windows, macOS | 1 |
| SSH, SFTP and forwarding integration | `cargo test` with `SHELLPP_TEST_SSHD=openssh` | `crates/ssh/tests` | Linux (Docker) | 2 |
| the same suite against the russh test server | `cargo test` with `SHELLPP_TEST_SSHD=russh` | `crates/ssh/tests` | Windows, macOS, Linux | 2 |
| sync end-to-end: two clients and a server | `cargo test`; SQLite always, PostgreSQL in Docker with `SHELLPP_TEST_POSTGRES=docker` | `server/tests` | Linux (CI), Windows (the full criterion of `CLAUDE.md`) | 7 |
| UI e2e | WebdriverIO + tauri-driver, `e2e-mocks` build | `apps/desktop/e2e` | Linux (xvfb, WebKitWebDriver), Windows (msedgedriver matching the installed WebView2) | 1 |
| macOS acceptance | build, unit and Rust integration tests, manual checklist | `docs/testing/macos-checklist.md` (from stage 1) | macOS | 1 |
| mobile build check | `cargo check` of `shellpp-core` and `shellpp-ssh` | section 2.3 | Linux (Android), macOS (iOS) | 1 |
| licenses and advisories | cargo-deny, `pnpm licenses:check` | repository | Linux | 0 (npm), 1 (cargo) |
| secret scan | gitleaks | repository | Linux | existing |

D6 sets the platform split; the test locations, the `SHELLPP_TEST_SSHD` switch and the macOS checklist path are a (stage 0 decision). The sync end-to-end tests live in `server/tests` because they need both sides, and only the server side may depend on the client crates.

### 9.2 CI jobs

Stage 0 has one Linux job for the frontend (D1). From stage 1 (stage 0 decision):

| Job | Runner | Steps |
|---|---|---|
| lint | Linux | rustfmt, clippy (both workspaces), `tsc`, eslint |
| licenses | Linux | cargo-deny (both workspaces), npm license check |
| linux | Linux | Rust tests, OpenSSH integration in Docker, UI e2e, Android check, packages (stage 9) |
| windows | Windows | Rust tests, russh integration, UI e2e, NSIS package (stage 9) |
| macos | macOS | build, Rust tests, russh integration, iOS check, `.dmg` (stage 9) |
| secret-scan | Linux | existing gitleaks workflow |
| server-deploy | Linux | the sync server's deployment tests in Docker (`server/tests/deploy.rs`: the image behind Caddy with two clients, the encrypted backup recipe end to end, the backup rotation, the compose file), only on changes to `server/`, `crates/sync-api`, `crates/core`, the root `Cargo.toml` and `rust-toolchain.toml`; its own workflow, `server-deploy.yml` (run 10) |
| server | Linux | the sync server's own tests (the server workspace, the sync end-to-end tests with two clients included) on SQLite, then on PostgreSQL in Docker (`SHELLPP_TEST_POSTGRES=docker`, a `postgres:17-alpine` container per test binary), on the same changes as server-deploy; its own workflow, `server.yml` (D202, run 11) |

### 9.3 Test servers

- OpenSSH (`tests/openssh/`): a Dockerfile on a pinned base image. Host keys, user keys and passwords are generated at test start (CONTRIBUTING, "Secrets and test data"). Users cover password, public key and keyboard-interactive authentication; the SFTP subsystem and TCP forwarding are enabled. Linux only (D6).
- russh (`tests/sshd/`, `shellpp-test-sshd`): password, public key and keyboard-interactive authentication; a deterministic built-in shell instead of the OS shell (it echoes input, can turn echo off to simulate a password prompt, and `exit <n>` ends with that status); SFTP over a temporary directory; `direct-tcpip` and `tcpip-forward`; switches to restart with a new host key and to drop connections, for the host key change and autoreconnect scenarios (stage 0 decision for the feature list).
- UI e2e uses the russh server on both Linux and Windows, so both platforms run the same scenarios with the same results. OpenSSH is used by the Rust integration suite (stage 0 decision).

### 9.4 Test rules

- Each e2e spec starts the app with a fresh temporary `SHELLPP_DATA_DIR` and a master password generated for the run.
- Elements are found by `data-testid` only (D22).
- Spec files are named after the UI section they cover, for example `e2e/specs/10.2-host-key.e2e.ts`; a script lists UI sections without a spec (stage 0 decision).
- KDF parameters are never weakened in tests: the client bounds forbid it ([security.md](security.md)). Instead, the crypto crates (`argon2`, `blake2`, `chacha20`, `poly1305`) are built with `opt-level = 3` in the dev and test profiles (`[profile.dev.package.<name>]`) (stage 0 decision).
- No real hosts, keys or credentials in tests or fixtures (CONTRIBUTING).

## 10. Packaging, updates and server deployment

### 10.1 Client packages and updates

Stage 9; summary of D7.

| OS | Package | In-app update | Notes |
|---|---|---|---|
| Windows | NSIS installer, per user | yes | registers `shellpp://` |
| macOS | `.dmg` | yes | Developer ID signing and notarization are the owner's decision; unsigned builds hide "remember on this device" and biometrics (D7, UI 13.4) |
| Linux | AppImage | yes | bundled LGPL libraries are listed in `THIRD_PARTY_NOTICES.md` (D4) |
| Linux | `.deb` | no, the package manager updates it | depends on the system WebKitGTK |

- `tauri-plugin-updater` with the project's own updater signing key. The public key is built into the app; the private key exists only as a CI secret. An update whose signature does not verify is not installed (UI 12.7). (Run 6, D97: the GPUI app only notifies, nothing is downloaded or installed; the in-app update of this table waits for the owner. Run 8, D170: the manifest it reads is signed with minisign and checked with the key built into the app; the release workflow `.github/workflows/release.yml` builds and signs the files of a draft release, docs/release.md.)
- The automatic check is off by default; when the user turns it on, it runs at launch. Help ▸ Check for updates and "Check now" always work (UI 1.4, 12.7).
- The check is a plain GET of a static manifest (default URL in section 7) with User-Agent `shellpp/<version>` (D97). The endpoint has no version, target or device placeholders; the manifest lists all platforms (D7).
- The update source is a local setting (Settings → Updates, UI 12.7).
- Installers register the URL scheme; development and e2e builds do not.

### 10.2 Server deployment

Stage 7; protocol and limits are in [sync-protocol.md](sync-protocol.md). (stage 0 decision except where SPEC is cited)

- Image `ghcr.io/proxyboy228/shellpp-server` with the static `shellpp-server` binary and CA certificates only, running as a non-root user.
- Subcommands: `serve`, `migrate`, `admin …` (users, registration codes, `purge`). `serve` refuses to start when the database schema is older than the binary; the compose file runs `migrate` as a one-shot service before `serve`. `serve` also runs the purge job once a day.
- `admin` also has `user quota` (per-account storage limits), `device revoke`, `vault delete`, and `restored`, which replaces `server_generation` at the end of every restore from a backup ([sync-protocol.md](sync-protocol.md), sections 3.4, 10.2 and 10.3). [decision] (D93)
- `server/docker-compose.yml`: `migrate`, `server` and PostgreSQL with a volume (SPEC). Small installations use SQLite with `SHELLPP_DATABASE_URL=sqlite://…` on a volume.
- The compose file also has a `backup` service that runs on a schedule (daily by default), pipes the dump (`pg_dump`, or an SQLite online backup on a tmpfs) straight into `age` to the operator's public key, and deletes files older than 30 days, the recommended retention; the private key is never on the server ([security.md](security.md), section 14.4). [decision] (D93)
- The server listens on plain HTTP at `SHELLPP_BIND`; TLS is terminated in front of it. The server is meant for a machine of the user's own, such as a homelab, and `server/README.md` covers three ways to reach it, each with an example configuration that the server's tests run ([sync-protocol.md](sync-protocol.md), section 2.2): [decision] (D93)
  - an own domain: Caddy as the reverse proxy with automatic Let's Encrypt certificates; `server/Caddyfile.example` (SPEC);
  - Cloudflare Tunnel: a `cloudflared` service in the compose file, no inbound port; Cloudflare's edge terminates TLS and sees the requests in the clear, record contents stay end-to-end encrypted ([security.md](security.md), RR23);
  - VPN only (Tailscale or WireGuard): the proxy listens only on the VPN address, with a certificate from an internal CA (such as Caddy's `tls internal`) that the user adds or pins in the client once per device.
- The client never accepts an invalid certificate silently: a certificate the system does not trust is used only after the user pinned it or added its CA, and a changed pinned certificate stops sync until the user confirms it; plain `http://` only for `localhost` ([sync-protocol.md](sync-protocol.md), sections 2 and 2.1). [decision] (D93)
- `GET /healthz` reports liveness and `GET /readyz` database readiness.
- Backup instructions (SPEC) live in `server/README.md`: the tested recipes above, restore into an empty database, `admin restored`, and the manual steps that undo what a restore brings back (`admin user delete`, `admin device revoke`, `admin vault delete`), and what a restore means for clients ([sync-protocol.md](sync-protocol.md), section 6.9; [security.md](security.md), section 14.4). [decision] (D93)

Environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `SHELLPP_DATABASE_URL` | none (required) | `postgres://…` or `sqlite://…` |
| `SHELLPP_BIND` | `127.0.0.1:8080` (Docker image: `0.0.0.0:8080`) | listen address |
| `SHELLPP_PUBLIC_URL` | none (required) | the https origin the clients use (with VPN-only access, the name inside the VPN), used in invitation links and emails. Keep it stable: clients bind account credentials to the origin they connect to ([security.md](security.md), section 14.4; [sync-protocol.md](sync-protocol.md), section 2.2) |
| `SHELLPP_SERVER_SECRET_FILE` | none (required, or the variable below) | a file holding the server secret: at least 32 bytes, base64; root of the server-only keys ([security.md](security.md)). The compose file hands it over as a Docker secret ([decision] (D177)) |
| `SHELLPP_SERVER_SECRET` | none | the secret itself instead of the file, for compatibility: it works with a warning in the log; set together with `SHELLPP_SERVER_SECRET_FILE` it is refused ([decision] (D177)) |
| `SHELLPP_REGISTRATION` | `invite` | `open` or `invite`. Invitations only by default; open registration is the operator's choice (D93) |
| `SHELLPP_QUOTA_MAX_RECORDS`, `SHELLPP_QUOTA_MAX_BYTES` | unset (no limit) | storage limits per account: live records and their envelope bytes; `admin user quota` sets them per account. Off by default; set them on an open server. [decision] (D93) |
| `SHELLPP_SMTP_URL`, `SHELLPP_SMTP_FROM` | unset | optional SMTP; without it only invitation links work and "verified" is hidden (D14) |
| `SHELLPP_TRUSTED_PROXIES` | unset (compose: `172.16.88.1/32`) | CIDR list whose `X-Forwarded-For` header is trusted: the one address the proxy that talks to the server reaches it from; with the compose file and a proxy on the host, the gateway of the compose network ([decision] (D176)) |
| `SHELLPP_TOMBSTONE_RETENTION_DAYS` | `90` | tombstone retention |
| `SHELLPP_RATE_LIMIT_*` | see [sync-protocol.md](sync-protocol.md) | rate limit overrides |
| `SHELLPP_LOG` | `info` | log level; logs carry user ids, never emails, keys, tokens or envelopes |
