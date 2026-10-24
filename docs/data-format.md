# Data format

This document describes the on-disk format that the code implements now, version 1: the files in the data directory, the SQLite tables, what is stored in the clear, the record envelope, the key wraps, the KDF and the device key in the OS keychain. Each point was checked against the code on 2026-10-20.

- The full design, including the parts that are not implemented yet (sync, team vaults, session recordings, exports), is [storage-format.md](storage-format.md). Where the two differ, this document describes the code.
- Byte order is big-endian. `be32(x)` is the 4-byte big-endian encoding of `x`, `be16(x)` the 2-byte one. `‖` is concatenation. Quoted labels such as `"shellpp/v1/kek"` are ASCII bytes without a terminator. `b64u` is base64url without padding.
- Sources: `crates/core/src/store` (schema in `migrations/0001_initial.sql`), `crates/core/src/crypto`, `crates/sync-api/src/envelope.rs`, `crates/core/src/vault`, `crates/core/src/lock`, `crates/core/src/service/lock_ops.rs`, `crates/core/src/settings`, `apps/desktop/src/dirs.rs` and `apps/desktop/src/platform/keychain.rs`.

## 1. Data directory

| OS | Data directory |
|---|---|
| Windows | `%LOCALAPPDATA%\shellpp\` (logs in `logs\`, temporary files in `tmp\` inside it) |
| macOS | `~/Library/Application Support/shellpp/` (logs in `~/Library/Logs/shellpp/`, temporary files in `~/Library/Caches/shellpp/tmp/`) |
| Linux | `$XDG_DATA_HOME/shellpp/`, by default `~/.local/share/shellpp/` (logs in `$XDG_STATE_HOME/shellpp/logs/`, temporary files in `$XDG_CACHE_HOME/shellpp/tmp/`) |

- `SHELLPP_DATA_DIR=<dir>` replaces all three with `<dir>`, `<dir>/logs` and `<dir>/tmp`. Test runs (a test build or `SHELLPP_TEST=1`) refuse to start without it, so they never use the permanent profile. `--demo` and `--screenshots` use a temporary profile.
- Unix: directories `0700`, the database file `0600`. Windows: the per-user profile permissions.

Files in the data directory:

| Path | Content |
|---|---|
| `shellpp.db` | the SQLite 3 database (section 2) |
| `shellpp.db-wal`, `shellpp.db-shm` | the SQLite write-ahead log and its index; part of the database |
| `shellpp.lock` | empty. The running app holds an exclusive OS file lock on it; a second process waits up to 3 seconds and then fails with a storage error |
| `backups/shellpp-<UTC yyyymmddThhmmssZ>.db` | a copy (`VACUUM INTO`) taken before a schema migration, the 3 newest kept. Schema 1 has no migrations, so none exist yet |
| `shellpp.db.<UTC yyyymmddThhmmssZ>.old`, with `-wal` and `-shm` | the database archived by "Start with a new vault" (section 6), or by log out with "delete local copy" (D190) |
| `shellpp.db.<UTC yyyymmddThhmmssZ>-failed.old`, with `-wal` and `-shm` | a new database set aside when "Start with a new vault" could not complete it (section 6); rare |
| `logs/shellpp.log`, `shellpp.log.1` … `.4` | diagnostic log, rotated at 5 MiB, when the logs directory is inside the data directory (Windows, `SHELLPP_DATA_DIR`). No secrets, addresses, usernames or paths |
| `tmp/` | temporary files of explicit user actions, emptied at start (no writer yet) |

## 2. SQLite database

- Crate `rusqlite` with bundled SQLite. Connection settings: `journal_mode = WAL`, `foreign_keys = ON`, `secure_delete = ON`, `synchronous = FULL`, `journal_size_limit = 0`, busy timeout 5 seconds.
- `PRAGMA user_version` is the schema version: `1`. A newer version is refused without touching the file; version 0 with tables in it is refused as not ours.
- `PRAGMA wal_checkpoint(TRUNCATE)` runs after every change of key material (password change, recovery, recovery key reissue, turning "remember on this device" off), before "Start with a new vault" and at exit, so replaced or deleted wraps do not stay in the WAL.

### 2.1 Tables (schema 1)

UUIDs are 16-byte `BLOB`s; timestamps are `TEXT` in RFC 3339 UTC with milliseconds (`2026-10-20T15:30:05.123Z`); booleans are `INTEGER` 0 or 1.

| Table | Columns | Written in version 1 |
|---|---|---|
| `meta` | `key TEXT PRIMARY KEY`, `value TEXT` | `db_id`, `device_id` (UUIDv7 text), `created_at` at the first run; `last_app_version` at every start; `unlock_lockout` (section 5). `account_user_id` and `account_server_url` are reserved for sync |
| `local_auth` | `id INTEGER PRIMARY KEY CHECK (id = 1)`, `personal_vault_id BLOB`, `kdf TEXT`, `kdf_salt BLOB`, `kdf_email TEXT NULL`, `updated_at TEXT` | one row after the first run: the KDF JSON (section 4.2), a 16-byte random salt, `kdf_email` `NULL` (no account) |
| `vaults` | `vault_id BLOB PRIMARY KEY`, `kind` (`personal`, `team`), `status` (`active`, `archived`), `role` (`owner`, `editor`, `viewer` or `NULL`), `current_epoch INTEGER`, `bound INTEGER DEFAULT 0`, `created_at TEXT` | one row: the personal vault, `active`, role `NULL`, epoch 1, `bound = 0` |
| `key_wraps` | `vault_id BLOB`, `epoch INTEGER`, `kind` (`kek`, `recovery`, `device`, `hpke`, `chain`), `sender_user_id BLOB NULL`, `hpke_enc BLOB NULL`, `wrap BLOB`; primary key (`vault_id`, `epoch`, `kind`) | `kek`, `recovery` and, with "remember on this device", `device` rows for the current epoch; `chain` rows only after a rotation (section 4.4). `hpke` is not used yet |
| `records` | `record_id BLOB`, `vault_id BLOB`, `class INTEGER`, `revision INTEGER`, `synced_revision INTEGER DEFAULT 0`, `key_epoch INTEGER`, `tombstone INTEGER DEFAULT 0`, `tombstoned_at TEXT NULL`, `server_seq INTEGER NULL`, `envelope BLOB`, `base_envelope BLOB NULL`; primary key (`vault_id`, `record_id`); unique index on `record_id`, indexes `records_by_class`, `records_pending` | one row per record (section 3) |
| `sync_state` | `vault_id`, `cursor`, `last_sync_at`, `last_error`, `resync_classes` | created, empty (no sync yet) |
| `server_account` | `id = 1`, `server_url`, `user_id`, `email`, `refresh_token`, `secrets_epoch`, `bound_at` | created, empty (no sync yet) |
| `local_settings` | `key TEXT PRIMARY KEY`, `value TEXT` (compact JSON) | a row only for a setting the user changed; a missing row means the default. Unknown keys and invalid values are ignored when read. Example: `security.auto_lock_minutes` = `null` (Never, the default), `5`, `15`, `30` or `60` (`1` is still accepted) |

### 2.2 What is stored in the clear

- Encrypted: `records.envelope` (and `base_envelope`, always `NULL` in version 1) and `key_wraps.wrap`. Everything a user typed (labels, addresses, usernames, passwords, keys, notes, tags, group structure) is inside an envelope.
- In the clear: every other column, including the KDF parameters, the salt, the vault and record ids, record classes, revisions, key epochs, the `meta` values, the unlock attempt counter and `local_settings` (interface preferences such as theme, layout and auto-lock; never secrets or host data).
- This reveals metadata: the number of records per class, how often a record was edited (revision), creation times (UUIDv7 ids) and padded sizes.


### 2.3 Schema 2 (stage 7, sync)

Migration 1 → 2 runs at the first start of a version with sync, after the backup of section 1 (`backups/shellpp-<time>.db`); it only adds, so a schema 1 file keeps every row (tested). `PRAGMA user_version` becomes `2`.

| Change | Columns | Written |
|---|---|---|
| `vaults` | `account_user_id BLOB NULL`, `server_url TEXT NULL` | the account and server a vault was last bound to |
| `server_account` | `server_generation BLOB NULL` | the server's generation seen at binding and after each restore |
| `recently_deleted` | `vault_id`, `record_id`, `class`, `deleted_at`, `origin` (`local`, `sync`), `envelope` | the last live envelope of a deleted record, 90 days, this device only (encrypted like any record) |
| `resync_returned` | `vault_id`, `record_id`, `returned_at` | records a full resync pushed back ("Brought back") |
| `duplicate_pairs` | `vault_id`, `record_id`, `other_record_id`, `found_at`, `kept` | possible duplicates raised by sync or the merge at sign-in |
| `server_tls` | `server_url`, `kind` (`pinned`, `ca`), `cert_sha256`, `ca_der`, `created_at` | the trust choice for a sync server (public data, readable while locked) |

With a bound account: `server_account.refresh_token` is a wrap blob of the refresh token under `local_secrets_key` (HKDF of the vault key with the device id and `secrets_epoch`), AAD `"shellpp/v1/wrap/refresh-token"` ‖ `device_id`; `local_auth.kdf_email` holds the account email; synced records keep `synced_revision`, `server_seq` and, while a local change is pending, `base_envelope`; deletions write tombstones. The setting `sync.toggles` (object: toggle → bool) chooses what the personal vault syncs on this device.

## 3. Records

### 3.1 Rows

- The columns `vault_id`, `record_id`, `class`, `revision`, `key_epoch` and `tombstone` are copies of the envelope header (section 3.2), written in the same statement. A row whose columns disagree with the header is refused on write and reported as damaged on read.
- Every write seals a new envelope with `revision + 1` (1 for a new record) and a fresh nonce, in one transaction.
- In version 1 the vault is never bound to a sync account (`bound = 0`) and no record was ever synced (`synced_revision = 0`, `server_seq` `NULL`), so a deletion removes the row; no tombstones are written. For undo, the core keeps the removed payload in memory for 60 seconds; the app offers Undo for the first 5 seconds (UI 2.4) and writes the record back under the same id. Nothing about the removed record is written to disk meanwhile.
- Records of an unknown envelope version, an unknown `type` or a newer payload `v` are kept unchanged and never rewritten; a record that fails a check is left out and reported as damaged, never deleted.

### 3.2 Envelope, version 1

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `version` = `0x01` |
| 1 | 1 | `class` (section 3.4) |
| 2 | 1 | `flags`: bit 0 = tombstone; bits 1–7 are 0 |
| 3 | 1 | reserved, `0x00` |
| 4 | 16 | `vault_id` |
| 20 | 16 | `record_id` (UUIDv7) |
| 36 | 8 | `revision`, u64 ≥ 1 |
| 44 | 4 | `key_epoch`, u32 ≥ 1 |
| 48 | 24 | nonce, random for every encryption |
| 72 | L + 16 | ciphertext ‖ 16-byte tag |

- The header is bytes `[0, 48)`. Total length = 88 + L, where L is the padded plaintext length, a multiple of 256 from 256 to 262 144; so an envelope is 344 to 262 232 bytes.
- AEAD: XChaCha20-Poly1305 (crate `chacha20poly1305`).
- Key: `record_key[v, e]` = HKDF-SHA256(`vault_key[v, e]`, info = `"shellpp/v1/record"` ‖ `vault_id` (16) ‖ `be32(e)`).
- AAD: `"shellpp/v1/record"` (17 bytes) ‖ header bytes `[0, 48)`, 65 bytes. A ciphertext moved to another record, vault, revision or epoch fails to open.
- Padding: P ‖ `0x80` ‖ `0x00`… up to L = 256 × ceil((len(P) + 1) / 256). The largest payload P is 262 143 bytes.

### 3.3 Payload

The plaintext P is compact UTF-8 JSON:

```json
{"v":1,"type":"host","hlc":"0199f6a1b2c30001","device_id":"0199f6a1-b2c3-7d4e-8f00-112233445566","data":{}}
```

| Field | Meaning |
|---|---|
| `v` | payload version of the type; `1` for every type so far |
| `type` | type name (section 3.4) |
| `hlc` | hybrid logical clock of the last change, 16 lowercase hex digits |
| `device_id` | the device of the last change |
| `data` | the fields of the type ([data-model.md](data-model.md)); absent in a tombstone |
| `deleted` | `true` only in a tombstone |

### 3.4 Classes and types

The class is the plaintext byte at offset 1; the exact type is only inside the payload. Codes `0x00` and `0x0B`–`0xFF` are reserved.

| Class | Code | Types the code reads and writes (type code) |
|---|---|---|
| `vault` | `0x01` | `vault_meta` (`0x0101`, `{"name": "Personal", "color": "graphite"}` at the first run), `recovery_key` (`0x0104`, section 4.5), `device_info` (`0x0106`: `device_id`, `name`, `platform`, `app_version`) |
| `hosts` | `0x02` | `host` (`0x0201`), `group` (`0x0202`) |
| `keys` | `0x03` | `key` (`0x0301`), `identity` (`0x0302`) |
| `history` | `0x06` | `history_entry` (`0x0601`), decrypted on first use |
| `connection_log` | `0x07` | `connection_log_entry` (`0x0701`), decrypted on first use |
| `known_hosts` | `0x08` | `known_host` (`0x0801`) |

`snippets` (`0x04`), `forwarding` (`0x05`), `appearance` (`0x09`) and `workspaces` (`0x0A`) are defined but not written yet.

## 4. Keys

### 4.1 Key chain

```text
master password ─NFC, UTF-8─► Argon2id(argon2_salt, kdf params) ─► master_key (32 B, memory only)
master_key ─HKDF "shellpp/v1/kek"─► kek ─opens─► key_wraps (personal, e, kek) ─► vault_key[personal, e]
device_key (OS keychain) ─HKDF "shellpp/v1/device-wrap" ‖ db_id─► device_wrap_key ─opens─► (personal, e, device)
recovery_key ─HKDF "shellpp/v1/recovery-kek"─► recovery_kek ─opens─► (personal, e, recovery)
vault_key[v, e] ─HKDF "shellpp/v1/record" ‖ vault_id ‖ be32(e)─► record_key[v, e] ─► envelopes
```

- HKDF is HKDF-SHA256 with an empty salt, `info` = label ‖ context and a 32-byte output.
- `vault_key[personal, 1]` is 32 random bytes from the OS CSPRNG, created at the first run.
- There is no password verifier on disk: a successful AEAD unwrap of the `kek` row proves the password.

### 4.2 KDF

`local_auth.kdf` holds compact JSON with the members in this order:

```json
{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4}
```

| Item | Value |
|---|---|
| Algorithm | Argon2id, version 0x13 (crate `argon2`), no secret, no associated data, 32-byte output |
| Desktop preset | `m_kib = 65536` (64 MiB), `t = 3`, `p = 4`; mobile preset `32768 / 4 / 2` |
| Accepted range | `alg = argon2id`, `v = 1`, `m_kib ≥ 32768`, `t ≥ 3`, `m_kib × t ≥ 131072`, `1 ≤ p ≤ 16`; desktop `m_kib ≤ 1048576`, mobile `m_kib ≤ 262144`, `t ≤ 16`. Anything else is refused before Argon2id runs |
| Argon2id salt | `argon2_salt` = SHA-256(`"shellpp/v1/kdf-salt"` ‖ `be16(len(e))` ‖ `e` ‖ `kdf_salt`), where `e` is the UTF-8 of `local_auth.kdf_email`, empty when it is `NULL` (always in version 1) |
| `kdf_salt` | 16 random bytes, new at the first run and at every password change |
| Password | the UTF-8 of the NFC-normalized password, not trimmed; at least 8 Unicode scalar values after NFC, no composition rules (D29) |

### 4.3 Wrap blob, version 1

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `version` = `0x01` |
| 1 | 24 | nonce, random |
| 25 | n + 16 | ciphertext ‖ tag (XChaCha20-Poly1305) |

A wrapped 32-byte key is 73 bytes.

### 4.4 `key_wraps` rows

| `kind` | Wrapped value | Wrapping key | AAD |
|---|---|---|---|
| `kek` | `vault_key[v, e]` | `kek` | `"shellpp/v1/wrap/kek"` ‖ `vault_id` ‖ `be32(e)` |
| `recovery` | `vault_key[v, e]` | `recovery_kek` | `"shellpp/v1/wrap/recovery"` ‖ `vault_id` ‖ `be32(e)` |
| `device` | `vault_key[v, e]` | `device_wrap_key` | `"shellpp/v1/wrap/device"` ‖ `vault_id` ‖ `be32(e)` |
| `chain` | `vault_key[v, e − 1]` | `chain_key[v, e]` = HKDF(`vault_key[v, e]`, `"shellpp/v1/epoch-chain"` ‖ `vault_id` ‖ `be32(e)`) | `"shellpp/v1/wrap/epoch-chain"` ‖ `vault_id` ‖ `be32(e − 1)` |

- `epoch` is the epoch of the wrapped key, except for `chain`, where it is the newer epoch `e`.
- `kek`, `recovery` and `device` rows exist only for `vaults.current_epoch`.
- A rotation of the personal vault (recovery with the recovery key, or a recovery key reissue) adds epoch `e + 1` with a new random key, its `chain` row and new `kek`, `recovery` and (when remembered) `device` rows, and deletes those of epoch `e`, in one transaction. Records are then re-sealed with the new epoch's key. The stage 1 interface offers neither operation, so a version 1 database normally has epoch 1 and no `chain` row.

### 4.5 Recovery key

- 32 random bytes, created at the first run. In local mode the app never shows it (D29), so it cannot be used to recover a forgotten password.
- It is kept twice: as the `recovery` wrap (section 4.4) and inside the encrypted record `recovery_key`, `data` = `{"recovery_key": b64u(32 bytes), "issued_at": timestamp}`. The text form shown with a sync account is `RK1-` plus 11 groups of 5 Crockford base32 characters (the 32 bytes and the first 2 bytes of their SHA-256).

### 4.6 Device key in the OS keychain

| Item | Value |
|---|---|
| Written when | "Remember on this device" is on: at the first run by default (D29), or when turned on in Settings |
| Service | `shellpp` |
| Account | `device-key:<db_id>`, `db_id` from `meta` as lowercase hyphenated UUID text |
| Secret | `device_key`, 32 random bytes, stored as b64u (43 characters) |
| Use | `device_wrap_key` = HKDF(`device_key`, `"shellpp/v1/device-wrap"` ‖ `db_id` (16 bytes)) opens the `device` row at launch |
| Removed when | the option is turned off: the item first, then the `device` row (and a WAL checkpoint). When the OS refuses to delete the item, nothing changes, the option stays on and Settings reports it. "Start with a new vault" keeps the item: it opens only the archived file |

- The app also reads the account `availability-probe` once to check that the keychain works; it never writes it. Unsigned macOS builds do not use the keychain at all.
- Tests, `--demo` and `--screenshots` use an in-memory secret store and never touch the OS keychain. In a test run (`cfg(test)` or `SHELLPP_TEST=1`) the OS keychain adapter also reports itself unavailable without calling the OS.
- Windows: the crate `keyring` writes a generic credential with `CRED_PERSIST_ENTERPRISE`. With a roaming user profile (domain Credential Roaming), the item travels to the other machines the user signs in to, so a copy of `shellpp.db` there opens without the master password too. Elsewhere, and on Windows without roaming, a copy of `shellpp.db` on another machine has no keychain item and needs the master password (`docs/security.md`, RR8).

## 5. Unlock attempt counter

`meta.unlock_lockout` holds plaintext JSON, written when an unlock attempt starts (the attempt counts as wrong until the password proves right) and reset to `{"failures":0}` by a successful unlock:

```json
{"failures":6,"retry_at_ms":1792510215000}
```

- `failures`: wrong master passwords in a row. `retry_at_ms`: Unix milliseconds when the current pause ends; absent before the fifth failure.
- The pause after the n-th failure is 5 s × 2^(n − 5), at most 300 s. The core refuses `unlock` before Argon2id runs while the pause lasts. Each attempt is counted as a failure when it starts, under the same lock as the check, and the count is taken back when the password proves right, so attempts running at the same time cannot get around the limit. A value that cannot be read counts as no failures; a pause end further away than the counter allows is cut back.

## 6. Archived database

"Start with a new vault" (UI 13.2) runs only while locked. It checkpoints and closes the database, keeping the lock on `shellpp.lock` (no other process can open the directory until the new database is in place), then renames:

| Before | After |
|---|---|
| `shellpp.db` | `shellpp.db.<UTC yyyymmddThhmmssZ>.old` |
| `shellpp.db-wal` | `shellpp.db.<UTC yyyymmddThhmmssZ>.old-wal` |
| `shellpp.db-shm` | `shellpp.db.<UTC yyyymmddThhmmssZ>.old-shm` |

- When that name is taken, `-2`, `-3` … up to `-999` comes before `.old`. If a rename fails, the renames already done are undone.
- Then a new, empty `shellpp.db` is created and the app is in first run. Nothing is deleted; `backups/` and the keychain item of the old `db_id` stay.
- If the new database cannot be created, the archive is put back. A half-made new database (with its side files) is not deleted either: it is first renamed to `shellpp.db.<UTC yyyymmddThhmmssZ>-failed.old` (`-2`, `-3` … before `.old` when taken).
- The archive is a complete database in the format of this document and still opens with its own master password, or at launch with its keychain item when that still exists. To go back to it, quit the app, move the current `shellpp.db` and its `-wal` and `-shm` files away, and rename the archive and its side files back to `shellpp.db`, `shellpp.db-wal` and `shellpp.db-shm`.

## 7. What changes when

| Event | Changes on disk |
|---|---|
| First run | `meta` ids, `local_auth`, the personal `vaults` row (epoch 1), `kek` and `recovery` rows, records `vault_meta`, `recovery_key` and `device_info`; then, with "remember on this device" and an OS keychain, the keychain item and the `device` row |
| Unlock | a successful unlock resets `meta.unlock_lockout`; after an app update, the `device_info` record gets the new `app_version` |
| Wrong password | `meta.unlock_lockout`, written when the attempt starts (before Argon2id) and reset when the password proves right |
| Record created, edited or removed | the `records` rows involved (removing a group also moves its children), in one transaction |
| Setting changed or reset | the `local_settings` row written or deleted |
| Password change | `local_auth` (new `kdf_salt`, current preset, `updated_at`) and the `kek` row; `recovery` and `device` rows unchanged |
| "Remember on this device" on / off | the keychain item and the `device` row, written or deleted together (off: the item first, then the row and a WAL checkpoint) |
| Start with a new vault | section 6 |
