# Storage format

This document defines how Shell++ stores data on a device and in the files it exports: the data directory, the local SQLite database, the encrypted record envelope, keys at rest, session recordings, temporary files, migrations, backups and every export format. It is written so that a third party can read their own data without the app ([principles 3 and 4](../SPEC.md)); section 14 is a step-by-step guide.

Related documents:

- [architecture.md](architecture.md): components, identifiers, directory policy, secrets trait, process model.
- [data-model.md](data-model.md): record types, the `data` fields of each type, references, import idempotency.
- [sync-protocol.md](sync-protocol.md): revisions, cursors, conflicts, key epochs, server schema.
- [security.md](security.md): threat model, key hierarchy, KDF, HKDF labels, HPKE, lock behavior, recovery key format.
- [decisions.md](decisions.md): accepted decisions, cited below as D1–D26.

Conventions:

- "(stage 0 decision)" marks a choice made while writing the stage 0 documents that is not in the specifications or the accepted decisions. A section heading marked this way covers its whole section.
- "[decision] (D93)" marks what was added from the owner's answers to the sync design draft (D93, 2026-10-22). The answers are the owner's; the details that carry them out are ours and can be changed, as D93 lists. A section heading marked this way covers its whole section.
- Byte order is big-endian everywhere. `be32(x)` is the 4-byte big-endian encoding of `x`, `be16(x)` the 2-byte one. `‖` is byte concatenation. Quoted labels such as `"shellpp/v1/kek"` are ASCII bytes without a terminator.
- `[a, b)` is a byte range from offset `a` inclusive to `b` exclusive.
- "UI x.y" is section x.y of [ui-behavior-spec.md](ui-behavior-spec.md); a bare "section x.y" is a section of this document.

## 1. Overview

| What | Where | Protection | Synced |
|---|---|---|---|
| Records (hosts, keys, snippets, history, log, …) | `records` table, one encrypted envelope per record | XChaCha20-Poly1305 per record (D10) | per class and toggle (D15) |
| Vault keys | `key_wraps` table | wrapped by KEK, recovery KEK, device key, HPKE or the epoch chain | wraps are exchanged through the sync API, not as records |
| KDF parameters and salt | `local_auth` table | plaintext (needed before unlock) | through the account API |
| Sync bookkeeping | `records` columns, `sync_state`, `vaults` | plaintext | no |
| Per-device settings | `local_settings` | plaintext, never secrets | no (D15) |
| Session recordings | `recordings/*.shpprec` | STREAM over XChaCha20-Poly1305 | never (D16) |
| Device key ("remember on this device") | OS keychain | OS keychain | never |
| SFTP Open temporary files | `tmp/` | none (explicit user action, D16) | never |
| Exports | user-chosen files | plain or encrypted (Argon2id + XChaCha20-Poly1305) | not applicable |

## 2. Data directory

### 2.1 Location per OS

The directory policy is owned by [architecture.md](architecture.md); this table repeats it for completeness. All directory names use `shellpp` (D2), not the bundle identifier.

| OS | Data (database, recordings, backups) | Diagnostic logs | Temporary files (SFTP Open) |
|---|---|---|---|
| Windows | `%LOCALAPPDATA%\shellpp\` | `%LOCALAPPDATA%\shellpp\logs\` | `%LOCALAPPDATA%\shellpp\tmp\` |
| macOS | `~/Library/Application Support/shellpp/` | `~/Library/Logs/shellpp/` | `~/Library/Caches/shellpp/tmp/` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/shellpp/` | `${XDG_STATE_HOME:-~/.local/state}/shellpp/logs/` | `${XDG_CACHE_HOME:-~/.cache}/shellpp/tmp/` |
| iOS, Android (later) | app sandbox, `shellpp/` under the app's support or files directory | same sandbox, `logs/` | same sandbox, `tmp/` |

- Windows uses the local (non-roaming) profile on purpose: the database belongs to one device.
- The environment variable `SHELLPP_DATA_DIR` replaces the data directory (tests and portable use).
- Unix permissions: directories `0700`, files `0600`. On Windows the per-user profile ACLs apply unchanged.
- One app process opens a data directory at a time (process model in [architecture.md](architecture.md)). The app enforces it with an exclusive OS file lock on `shellpp.lock` (section 2.2), also where the single-instance check is not available; a second process waits a few seconds for the lock (a restart), then reports a storage error.

### 2.2 Files in the data directory

| Path | Content | Format |
|---|---|---|
| `shellpp.db` | the database | SQLite 3, WAL mode (section 4) |
| `shellpp.db-wal`, `shellpp.db-shm` | SQLite write-ahead log and shared-memory index | SQLite; part of the database while the app runs |
| `shellpp.lock` | empty; the running app holds an exclusive OS file lock on it, so a second process refuses to open the directory (section 2.1) | empty file |
| `recordings/<file_id>.shpprec` | encrypted session output | section 8 |
| `recordings/<file_id>.<vault_id>.rekey` | the key of a recording that waits to be re-encrypted into the personal vault `<vault_id>`, wrapped for that vault | section 8.4 |
| `backups/shellpp-<UTC yyyymmddThhmmssZ>.db` | copy taken before each schema migration; the last 3 are kept | SQLite (section 12) |

`<file_id>` is the lowercase hyphenated UUID text. Diagnostic logs (`shellpp.log`, rotated at 5 MiB, 5 files kept) never contain secrets, host addresses, usernames, command text, terminal data or file names ([architecture.md](architecture.md), section 3.8); they are not user data and are not described further here.

### 2.3 OS keychain item

Only when "remember on this device" is on (D8; on by default since D29, see [data-format.md](data-format.md)):

| Attribute | Value |
|---|---|
| Service | `shellpp` |
| Account | `device-key:<db_id>` (`db_id` as lowercase hyphenated UUID text) |
| Secret | `device_key`, 32 random bytes, stored as base64url without padding (43 characters) (stage 0 decision) |

Section 6.3 describes how the device key is used.

## 3. Encoding conventions

| Item | Encoding |
|---|---|
| UUIDs | RFC 9562. In binary structures and SQLite `BLOB` columns: 16 raw bytes in network order. In JSON and file names: lowercase hyphenated text. |
| Ids generated by the client | `record_id`, `vault_id`, `device_id`, `db_id`, recording `file_id`: UUIDv7. `user_id` is a UUIDv7 generated by the server. |
| Integers in binary structures | unsigned big-endian |
| Binary values in JSON | base64url without padding (RFC 4648 §5), written `b64u` |
| Timestamps in JSON and SQLite `TEXT` columns | RFC 3339 UTC with milliseconds, for example `2026-10-18T22:31:05.123Z` |
| JSON field names | `snake_case`; the terminal theme format keeps the camelCase names fixed in D25 |
| Passwords before a KDF | UTF-8 of the NFC-normalized string, no trimming |
| Email | trimmed, Unicode NFC, lowercase |
| Booleans in SQLite | `INTEGER` 0 or 1 |
| JSON numbers | integers only, in the range ±(2^53 − 1), so every JSON reader keeps them exact (stage 0 decision) |
| `revision` | u64 by definition; values stay below 2^63 because SQLite and PostgreSQL store signed 64-bit integers (stage 0 decision) |

## 4. SQLite database

### 4.1 Connection settings

- Crate `rusqlite` with feature `bundled` (SQLite is public domain, D4).
- `PRAGMA journal_mode = WAL`, `PRAGMA foreign_keys = ON`, `PRAGMA secure_delete = ON` (freed pages are overwritten with zeros).
- `PRAGMA synchronous = FULL`, because key material changes (password change, recovery) must survive a power loss once the server has accepted them (stage 0 decision).
- `PRAGMA journal_size_limit = 0`, and `PRAGMA wal_checkpoint(TRUNCATE)` right after every change of key material (password change, recovery with the recovery key, recovery key reissue) and at exit, so replaced key wraps do not stay in `shellpp.db-wal` (`secure_delete` covers only the database file).
- `PRAGMA user_version` holds the schema version (section 11). The schema below is version 1.

### 4.2 Schema, version 1

Stage 1 migrations may add columns, indexes and tables but must not rename the tables and columns below.

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,        -- keys: section 4.5
  value TEXT NOT NULL
);

CREATE TABLE local_auth (         -- exactly one row after first run
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  personal_vault_id BLOB NOT NULL,
  kdf               TEXT NOT NULL, -- KDF JSON, section 6.5
  kdf_salt          BLOB NOT NULL, -- 16 bytes
  kdf_email         TEXT,          -- normalized email bound into the Argon2id salt; NULL until first bound to an account
  updated_at        TEXT NOT NULL
);

CREATE TABLE vaults (
  vault_id      BLOB PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('personal', 'team')),
  status        TEXT NOT NULL CHECK (status IN ('active', 'archived')),
  role          TEXT CHECK (role IN ('owner', 'editor', 'viewer')),
  current_epoch INTEGER NOT NULL,
  bound         INTEGER NOT NULL DEFAULT 0, -- 1 = synced with the bound account
  created_at    TEXT NOT NULL
);

CREATE TABLE key_wraps (
  vault_id       BLOB NOT NULL REFERENCES vaults (vault_id),
  epoch          INTEGER NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('kek', 'recovery', 'device', 'hpke', 'chain')),
  sender_user_id BLOB,           -- hpke only
  hpke_enc       BLOB,           -- hpke only, 32 bytes
  wrap           BLOB NOT NULL,  -- wrap blob, or HPKE ct for 'hpke'
  PRIMARY KEY (vault_id, epoch, kind)
);

CREATE TABLE records (
  record_id       BLOB NOT NULL,
  vault_id        BLOB NOT NULL REFERENCES vaults (vault_id),
  class           INTEGER NOT NULL,
  revision        INTEGER NOT NULL,
  synced_revision INTEGER NOT NULL DEFAULT 0,
  key_epoch       INTEGER NOT NULL,
  tombstone       INTEGER NOT NULL DEFAULT 0,
  tombstoned_at   TEXT,          -- local time the tombstone was written or received
  server_seq      INTEGER,
  envelope        BLOB NOT NULL, -- section 5, byte-identical to the server copy
  base_envelope   BLOB,          -- pending records only: the synced envelope this version derives from
  PRIMARY KEY (vault_id, record_id)
);
CREATE UNIQUE INDEX records_record_id ON records (record_id);
CREATE INDEX records_by_class ON records (vault_id, class);
CREATE INDEX records_pending ON records (vault_id) WHERE revision <> synced_revision;

CREATE TABLE sync_state (
  vault_id       BLOB PRIMARY KEY REFERENCES vaults (vault_id),
  cursor         INTEGER NOT NULL DEFAULT 0,
  last_sync_at   TEXT,
  last_error     TEXT,           -- error code only
  resync_classes TEXT            -- JSON array of class codes awaiting a full pull
);

CREATE TABLE server_account (     -- zero or one row
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  server_url    TEXT NOT NULL,
  user_id       BLOB NOT NULL,
  email         TEXT NOT NULL,
  refresh_token BLOB,             -- wrap blob under local_secrets_key; NULL while signed out after an offline recovery (section 6.4)
  secrets_epoch INTEGER NOT NULL, -- personal vault epoch used for local_secrets_key
  bound_at      TEXT NOT NULL
);

CREATE TABLE local_settings (     -- per-device settings, JSON values
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL             -- never secrets, host data or command text
);
```

Added with stage 7 by an additive migration (section 11) ([decision] (D93)):

```sql
ALTER TABLE vaults ADD COLUMN account_user_id BLOB;  -- the account the vault was last bound to; kept when archived (D93, Choice 4 (a))
ALTER TABLE vaults ADD COLUMN server_url TEXT;       -- normalized origin of that account's server
ALTER TABLE server_account ADD COLUMN server_generation BLOB; -- last `server_generation` seen (sync-protocol 3.4)

CREATE TABLE recently_deleted (   -- data-model 16.6; never synced
  vault_id   BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id  BLOB NOT NULL,
  class      INTEGER NOT NULL,
  deleted_at TEXT NOT NULL,       -- local time the tombstone was written or applied
  origin     TEXT NOT NULL CHECK (origin IN ('local', 'sync')),
  envelope   BLOB NOT NULL,       -- byte copy of the last live envelope (section 5)
  PRIMARY KEY (vault_id, record_id)
);
CREATE INDEX recently_deleted_by_time ON recently_deleted (deleted_at);

CREATE TABLE resync_returned (    -- data-model 16.7: records a full resync pushed back
  vault_id    BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id   BLOB NOT NULL,
  returned_at TEXT NOT NULL,
  PRIMARY KEY (vault_id, record_id)
);

CREATE TABLE duplicate_pairs (    -- data-model 16.8
  vault_id        BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id       BLOB NOT NULL,
  other_record_id BLOB NOT NULL,
  found_at        TEXT NOT NULL,
  kept            INTEGER NOT NULL DEFAULT 0, -- 1 = "Keep both"
  PRIMARY KEY (vault_id, record_id, other_record_id)
);

CREATE TABLE server_tls (         -- sync-protocol 2.1: trust for a server the system does not trust
  server_url  TEXT PRIMARY KEY,   -- normalized origin
  kind        TEXT NOT NULL CHECK (kind IN ('pinned', 'ca')),
  cert_sha256 BLOB NOT NULL,      -- 'pinned': of the server certificate; 'ca': of the CA certificate (DER)
  ca_der      BLOB,               -- 'ca' only: the CA certificate
  created_at  TEXT NOT NULL
);
```

### 4.3 Tables

| Table | Role | Notes |
|---|---|---|
| `meta` | identity of this installation and app bookkeeping | keys in section 4.5 |
| `local_auth` | KDF parameters, salt and salt-bound email for the master password; id of the personal vault | KDF JSON in section 6.5. Updated atomically with the `kek` wrap on a password change, an email change, registration and sign-in. |
| `vaults` | every vault this device holds | `role` is `NULL` for the personal vault. `archived` = read-only local archive of a team vault the user lost access to (D14), or of the personal vault of an earlier account ([data-model.md](data-model.md), section 18.5). `bound = 1` while the vault syncs with the bound account. `account_user_id` and `server_url` name the account it was last bound to and stay when it is archived or unbound ([decision] (D93)). |
| `key_wraps` | wrapped vault keys | section 6.2 |
| `records` | one row per record: the envelope plus a plaintext copy of its header fields | section 4.6 |
| `sync_state` | one pull cursor per vault (D12) | semantics in [sync-protocol.md](sync-protocol.md) |
| `server_account` | the bound sync account, if any | the refresh token is wrapped (section 6.4); access tokens are kept in memory only |
| `local_settings` | per-device app settings (D15), JSON values | never secrets, host data (addresses, usernames) or command text; anything that contains them is a record |
| `recently_deleted` | the last live envelope of each deleted record, 90 days ([data-model.md](data-model.md), section 16.6) | ciphertext; never synced; not part of Export Data (only the screen's own Export writes chosen entries, [data-model.md](data-model.md), section 16.6). [decision] (D93) |
| `resync_returned` | records a full resync pushed back, for review ([data-model.md](data-model.md), section 16.7) | ids and times only. [decision] (D93) |
| `duplicate_pairs` | pairs of records with the same natural key from two devices ([data-model.md](data-model.md), section 16.8) | ids, times and "Keep both". [decision] (D93) |
| `server_tls` | the pinned certificate or the added CA of a sync server the system does not trust ([sync-protocol.md](sync-protocol.md), section 2.1) | public data; read while locked by the recovery. [decision] (D93) |

There is no separate migrations table: `PRAGMA user_version` is the schema version and the pre-migration copies in `backups/` are the history (section 11). There is no local devices table: this installation is identified by `meta` (section 4.5), and the other devices of an account are known from the server's device list and from `device_info` records ([data-model.md](data-model.md)).

### 4.4 What is stored in the clear

Only what is needed before unlock or for sync bookkeeping is plaintext (D10). Everything else, including names, addresses, usernames, passwords, keys, notes, tags, scripts, commands, log contents, group structure, vault names, exact record types and HLC values, is inside an envelope or a wrap blob.

| Column | Why it is plaintext |
|---|---|
| `records.record_id`, `records.vault_id` | addressing, foreign keys and sync; the server sees them anyway |
| `records.class` | sync class filter (D15) and lazy decryption of `history` and `connection_log` (D10); the server sees it |
| `records.revision`, `synced_revision`, `server_seq` | optimistic concurrency and pending-change detection (D12) |
| `records.key_epoch` | selects the vault key epoch before decryption (D14) |
| `records.tombstone`, `tombstoned_at` | tombstone purge after 90 days without decrypting (D12) |
| `vaults.*` | which vaults exist, their kind, status, role, current epoch and bound flag: needed to choose and open wraps at unlock and to sync |
| `key_wraps.vault_id`, `epoch`, `kind`, `sender_user_id`, `hpke_enc` | addressing of the wraps; `wrap` itself is ciphertext |
| `local_auth.*` | the KDF parameters, the salt and the email bound into the Argon2id salt are inputs to the key derivation, so they cannot be encrypted under its output. `kdf_email` stays after log out and account deletion, because the local copy still opens with keys derived from it |
| `sync_state.*` | cursors, error codes and class lists |
| `vaults.account_user_id`, `vaults.server_url`, `server_account.server_generation` | needed to open an archived team vault's HPKE wrap and to detect a server restore; the server knows them anyway. [decision] (D93) |
| `recently_deleted.vault_id`, `record_id`, `class`, `deleted_at`, `origin` | the 90-day purge and the list without decrypting; the record itself stays an envelope. [decision] (D93) |
| `resync_returned.*`, `duplicate_pairs.*` | ids, times and a flag: the lists of [data-model.md](data-model.md), sections 16.7 and 16.8. [decision] (D93) |
| `server_tls.*` | the trust choice is needed before unlock (the recovery from the unlock screen calls the server); a certificate and its hash are public. A local attacker can replace it ([security.md](security.md), RR22). [decision] (D93) |
| `server_account.server_url`, `user_id`, `email`, `secrets_epoch`, `bound_at` | needed while locked: the "forgot password" recovery flow calls the server with the email before anything can be decrypted. The dialog pre-fills the email and lets the user edit it, because an email change made on another device signs this device out without updating this column ([sync-protocol.md](sync-protocol.md), section 5.2). The refresh token is wrapped. |
| `meta.*` | ids and the app version: needed before unlock and for migrations |
| `local_settings.*` | settings the locked app needs (theme, auto-lock, window layout, …) |

The plaintext still reveals metadata: the number of records per class and vault, edit counts (revisions), creation times (UUIDv7) and padded sizes. [security.md](security.md) lists this as accepted local metadata.

### 4.5 Installation identity

| `meta` key | Value | Written |
|---|---|---|
| `db_id` | UUIDv7 text: identity of this database file; names the keychain account and is the context of `device_wrap_key` | first run |
| `device_id` | UUIDv7 text: the device identity used for HLC tie-breaks, the server's device list and `local_secrets_key` | first run; replaced before binding to a different account than the last bound one (lifecycle in [architecture.md](architecture.md), section 3.2) |
| `created_at` | timestamp of the first run | first run |
| `last_app_version` | semver of the last app version that opened the database | every start |
| `account_user_id`, `account_server_url` | `user_id` (UUID text) and server origin of the last bound account | at binding; kept after log out and account deletion (stage 0 decision) |
| `sync_reset_vault` | the vault whose "Reset sync state" resync has not completed yet (UUID text), empty when none ([sync-protocol.md](sync-protocol.md), section 6.4, step 2) | at the user's confirmation; emptied when that resync completes ([decision] (D183)) |

The last two keys exist because an archived team vault must stay readable after the `server_account` row is gone (principle 3): opening its HPKE wrap needs the recipient's `user_id` in the HPKE `info`, and the sender's public key is found in a `member_pin` record by (`server_url`, `user_id`) (section 14.5). They name only the last account, so since D93 every `vaults` row also keeps the `account_user_id` and `server_url` it was bound to (section 4.2): an archive of an earlier account stays readable after the device binds to another one, with that account's key pair and pins from its archived personal vault. The `meta` keys stay as the fallback for rows that predate the migration. [decision] (D93; Choice 4 (a))

### 4.6 The `records` table

- The `envelope` column is authoritative and byte-identical to the server copy (D10). The columns `vault_id`, `record_id`, `class`, `revision`, `key_epoch` and `tombstone` are copies of the envelope header (section 5.1), written in the same statement. A row whose columns disagree with its header is corrupt.
- A record's identity is (`vault_id`, `record_id`), the primary key; pulls, pushes, conflict resolution and the in-memory index always look records up by both. `record_id` is also unique on the device (index `records_record_id`, [data-model.md](data-model.md), section 3.1): a pulled record whose `record_id` belongs to another local vault is rejected as an integrity failure ([sync-protocol.md](sync-protocol.md), section 6.4), and the unique index makes a faulty implementation fail instead of overwriting. (stage 0 decision)
- `synced_revision` is the revision last acknowledged by or received from the server; `0` means the server never had the record. A record is pending when `revision <> synced_revision`.
- `server_seq` is the server sequence number of the version last seen; `NULL` until the record is synced.
- `base_envelope` (stage 0 decision) holds, while a record with `synced_revision > 0` is pending, a byte copy of the synced envelope its current version derives from: the envelope with revision `synced_revision`, or the server version taken as the new base after a conflict ([sync-protocol.md](sync-protocol.md), section 6.7). Its payload gives the base stamp `(hlc, device_id)` of [sync-protocol.md](sync-protocol.md), section 6.1. It is set at the first local write after a sync (including a background re-encryption) and set to `NULL` when the record becomes synced; it is `NULL` for records that were never synced. A synced record that a full resync sets back to `synced_revision = 0` keeps its own synced envelope here, so a server version that turns up later wins without a conflict copy ([sync-protocol.md](sync-protocol.md), section 6.9). Being ciphertext, it keeps HLC values out of the clear (section 4.4).
- On unlock, records are decrypted into an in-memory index. Classes `history` (`0x06`) and `connection_log` (`0x07`) are decrypted lazily, on first use (D10). Search and sorting run in memory.

Local writes (stage 0 decision for the points not covered by [sync-protocol.md](sync-protocol.md)):

- Every write (edit, deletion, re-encryption) produces a new envelope with `revision + 1` and a fresh nonce; the row is replaced in one transaction.
- A deletion writes a tombstone: an envelope with the tombstone flag set and the payload `{"v": 1, "type": …, "hlc": …, "device_id": …, "deleted": true}`.
- Exception: when the vault has `bound = 0` and the record has `synced_revision = 0`, nothing has to be propagated, so a deletion removes the row at once.
- Writing a tombstone for a live record, or applying a pulled one, also writes the record's last live envelope into `recently_deleted` in the same transaction, for the types [data-model.md](data-model.md), section 16.6, keeps; the exception above keeps that copy too. Undo, Restore and a later live revision remove the entry. [decision] (D93)
- Tombstones that the server has acknowledged are purged after 90 days (D12).
- A row is deleted only by a local action, by the purge of an old tombstone or by applying a received tombstone. A record missing from a server snapshot is kept ([sync-protocol.md](sync-protocol.md), section 6.9).
- No record can be written while the app is locked, because record keys are wiped (D9). Data produced while locked, such as the end time of a connection log entry, is held in memory and written after the next unlock; it is lost if the app exits while locked.
- A record that fails any check in section 5.4 keeps its row unchanged and is left out of the in-memory index. It is reported as damaged and never deleted automatically (principle 3).

## 5. Encrypted record envelope

### 5.1 Byte layout (version 1)

(D10, D14; layout chosen in stage 0)

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | `version` | `0x01` |
| 1 | 1 | `class` | u8, record class ([data-model.md](data-model.md)) |
| 2 | 1 | `flags` | bit 0 = tombstone; bits 1–7 reserved, must be 0 |
| 3 | 1 | `reserved` | `0x00` |
| 4 | 16 | `vault_id` | UUID bytes |
| 20 | 16 | `record_id` | UUIDv7 bytes |
| 36 | 8 | `revision` | u64, ≥ 1 |
| 44 | 4 | `key_epoch` | u32, ≥ 1 |
| 48 | 24 | `nonce` | random, from the OS CSPRNG for every encryption |
| 72 | L + 16 | ciphertext ‖ tag | L = padded plaintext length, a multiple of 256 |

- The header is bytes `[0, 48)`; the fixed part is 72 bytes; total length = `88 + L`.
- Minimum envelope: 344 bytes (L = 256). Maximum: 262 232 bytes (L = 262 144 = 256 KiB).

Record classes, for reference (owned by [data-model.md](data-model.md)):

| Code | Class | Code | Class |
|---|---|---|---|
| `0x01` | `vault` | `0x06` | `history` |
| `0x02` | `hosts` | `0x07` | `connection_log` |
| `0x03` | `keys` | `0x08` | `known_hosts` |
| `0x04` | `snippets` | `0x09` | `appearance` |
| `0x05` | `forwarding` | `0x0A` | `workspaces` |

Codes `0x00` and `0x0B`–`0xFF` are reserved. The exact type (u16 = `class << 8 | kind`) is inside the ciphertext.

### 5.2 Key and AAD

| Item | Value |
|---|---|
| AEAD | XChaCha20-Poly1305 (crate `chacha20poly1305`), 32-byte key, 24-byte nonce, 16-byte tag appended to the ciphertext |
| Key | `record_key[vault_id, key_epoch]` = HKDF-SHA256(IKM = `vault_key[vault_id, key_epoch]`, salt = empty, info = `"shellpp/v1/record"` ‖ `vault_id` (16) ‖ `be32(key_epoch)`, L = 32) |
| Nonce | envelope bytes `[48, 72)` |
| AAD | `"shellpp/v1/record"` (17 ASCII bytes) ‖ envelope bytes `[0, 48)`, 65 bytes in total |
| Plaintext | the padded payload (sections 5.3 and 5.5) |

The AAD binds version, class, flags, vault id, record id, revision and key epoch, so a ciphertext moved to another record, vault, revision or epoch fails authentication (D14). The same label string is used as the HKDF `info` prefix and as the AAD prefix; the two uses are in different primitives and do not interact.

### 5.3 Padding

(D10; scheme chosen in stage 0)

- P is the UTF-8 payload JSON.
- `L = 256 × ceil((len(P) + 1) / 256)`.
- padded = P ‖ `0x80` ‖ `0x00` × (L − len(P) − 1) (ISO/IEC 7816-4 style).
- Unpadding strips trailing `0x00` bytes and then requires and strips one `0x80`; anything else is a decryption error.
- Maximum `len(P)` = 262 143 bytes. A write whose payload is larger is refused with a "too large" error; a history entry that would be larger is not stored.

### 5.4 Checks on read

In this order; any failure makes the record damaged (section 4.6):

1. `version = 0x01`. An unknown version means "written by a newer app": the row is kept, the record is shown as unsupported and is never rewritten.
2. `len ≥ 344`, `len ≤ 262 232`, `(len − 88) mod 256 = 0`.
3. `class` is known, `flags` bits 1–7 are 0, `reserved = 0x00`.
4. Header `vault_id`, `record_id`, `revision`, `key_epoch` and the tombstone bit equal the row columns.
5. AEAD decryption succeeds with the key, nonce and AAD from section 5.2.
6. Unpadding succeeds; P is valid UTF-8 JSON and an object.
7. The class of the payload `type` equals the envelope `class` (`conflict` copies have the class of their original).
8. The tombstone flag is set if and only if the payload has `"deleted": true`.

### 5.5 Plaintext payload

```json
{
  "v": 1,
  "type": "host",
  "hlc": "0199f6a1b2c30001",
  "device_id": "0199f6a1-b2c3-7d4e-8f00-112233445566",
  "data": {}
}
```

| Field | Type | Meaning |
|---|---|---|
| `v` | integer ≥ 1 | schema version of this payload (section 5.6) |
| `type` | string | type name, for example `host`, `key`, `history_entry`, `conflict` ([data-model.md](data-model.md)) |
| `hlc` | string, 16 lowercase hex digits | hybrid logical clock of the last semantic change ([sync-protocol.md](sync-protocol.md)) |
| `device_id` | UUID text | device of the last semantic change |
| `data` | object | type-specific fields ([data-model.md](data-model.md)); absent in a tombstone |
| `deleted` | `true` | present only in a tombstone |

- Serialization: UTF-8 without a byte order mark, compact (no insignificant whitespace). Key order carries no meaning; comparisons are made on parsed JSON values, never on bytes.
- A conflict copy has `"type": "conflict"` and `data` = `{original_record_id, original_type, original_revision, losing_hlc, losing_device_id, losing_deleted, losing_data}`.
- The plain export contains the same objects (section 13.2), so the payload schema and the export schema are one schema.

### 5.6 Versioning and forward compatibility

(stage 0 decision)

- `v` versions the payload of its `type`: a reader selects the schema by the pair (`type`, `v`). Every type starts at `v = 1`; a type's `v` is raised only for an incompatible change of its `data`, independently of other types. [data-model.md](data-model.md) lists the current `v` of each type.
- Compatible additions (a new optional field) do not change `v`. Writers keep `data` fields they do not know when they rewrite a record, so an older app does not erase fields written by a newer one.
- A record with a higher `v` than the app knows, or an unknown `type`, is kept unchanged, shown as "requires a newer version" and never rewritten by that app.
- A payload is upgraded to a newer `v` only as part of a semantic edit. There is no bulk rewrite after an app update, so an update causes no sync traffic and no spurious conflicts.

## 6. Keys at rest

The key hierarchy, the KDF and the HKDF labels are owned by [security.md](security.md). This section defines how wrapped keys are stored.

### 6.1 Wrap blob (version 1)

Used for every key wrap except HPKE, and for small local secrets. (stage 0 decision)

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `version` = `0x01` |
| 1 | 24 | `nonce`, random |
| 25 | n + 16 | ciphertext ‖ tag (XChaCha20-Poly1305) |

For a 32-byte key, n = 32 and the blob is 73 bytes.

### 6.2 `key_wraps` rows

| `kind` | Vault kinds | Wrapped value | Wrapping key | AAD | `wrap` column |
|---|---|---|---|---|---|
| `kek` | personal | `vault_key[v, e]` | `kek` | `"shellpp/v1/wrap/kek"` ‖ `vault_id` ‖ `be32(e)` | 73-byte wrap blob |
| `recovery` | personal | `vault_key[v, e]` | `recovery_kek` | `"shellpp/v1/wrap/recovery"` ‖ `vault_id` ‖ `be32(e)` | 73-byte wrap blob |
| `device` | personal, optional | `vault_key[v, e]` | `device_wrap_key` | `"shellpp/v1/wrap/device"` ‖ `vault_id` ‖ `be32(e)` | 73-byte wrap blob |
| `hpke` | team | `vault_key[v, e]` | HPKE to the user's X25519 key | HPKE `info` (section 14.5); `aad` empty | HPKE `ct`, 48 bytes; `hpke_enc` holds `enc` (32 bytes) and `sender_user_id` the sender |
| `chain` | personal, team | `vault_key[v, e − 1]` | `chain_key[v, e]` | `"shellpp/v1/wrap/epoch-chain"` ‖ `vault_id` ‖ `be32(e − 1)` | 73-byte wrap blob |

- `epoch` is the epoch `e` of the key that is wrapped, except for `chain`, where `epoch` is `e`, the newer epoch whose chain key wraps `e − 1`; this matches the server's `prev_key_wrap[v, e]` ([sync-protocol.md](sync-protocol.md)).
- `kek`, `recovery`, `device` and `hpke` rows are kept only for the vault's `current_epoch`. When a new epoch arrives, its rows are written and the older ones deleted in the same transaction. `chain` rows are kept for every epoch from 2 to `current_epoch`, so every older key can be reached from the newest one (stage 0 decision).
- Epochs never go back (stage 0 decision): `vaults.current_epoch` is never lowered, and a row of an epoch at or below the local current epoch is never replaced by one whose wrapped key differs (a rewrap of the same key under another wrapping key, as after a password change or a new `recovery_key` revision, stays allowed); a new epoch is always above the local current epoch. Otherwise a server restored from a backup, or one replaying an older genuine wrap, could make the device reuse an epoch number it holds, overwrite that epoch's `kek`, `recovery` or `chain` row and lose every record sealed under the newer local keys. For the personal vault, a server answer that reports a lower epoch, or a wrap of a held epoch that opens to another key, changes nothing and shows "server state rolled back" ([sync-protocol.md](sync-protocol.md), section 5.1); a recovery on that device then opens the local `recovery` row and only rewraps locally, without a rotation. Team vault epochs move only forward through epoch acceptance ([sync-protocol.md](sync-protocol.md), section 7.7). A team vault whose server epoch is below the local one changes no row either. Bringing a server back to the local epochs (the re-post of [sync-protocol.md](sync-protocol.md), sections 5.1 and 7.7) sends the `chain` rows this device holds as they are and wraps the keys it holds again for the server; it writes no local key row. [decision] (D93)
- The `recovery` row is built by each device from the synced `recovery_key` record ([data-model.md](data-model.md), section 6.7) while unlocked, and rebuilt whenever a new revision of that record is applied or the personal vault gets a new epoch. It is never copied from the server's `recovery_wrap`, which the client only checks against it ([sync-protocol.md](sync-protocol.md), section 4.4). A device that has not yet pulled the record has no `recovery` row. (stage 0 decision)
- The old personal vault of an earlier account, kept as an archive when the device binds to another account ([data-model.md](data-model.md), section 18.5), keeps `kek`, `recovery` and `device` rows for its last epoch under the same keys as the current personal vault's rows. They are rewritten in the same transaction whenever the current personal vault's rows of that kind are, so the archive opens wherever the current personal vault does. (stage 0 decision)
- There is no password verifier on disk: a successful AEAD unwrap of the current personal vault key proves the password.
- Derived keys used here: `kek` = HKDF(`master_key`, `"shellpp/v1/kek"`); `recovery_kek` = HKDF(`recovery_key`, `"shellpp/v1/recovery-kek"`); `device_wrap_key` = HKDF(`device_key`, `"shellpp/v1/device-wrap"` ‖ `db_id`); `chain_key[v, e]` = HKDF(`vault_key[v, e]`, `"shellpp/v1/epoch-chain"` ‖ `vault_id` ‖ `be32(e)`). All HKDF calls are HKDF-SHA256 with an empty salt and a 32-byte output.

### 6.3 Device key ("remember on this device")

- On by default since D29 and set on the first-run screen; turning it on later shows a warning (D8). It is unavailable without an OS keychain, for example without Secret Service on Linux (D5), and in unsigned macOS builds (D7).
- Turning it on: generate `device_key` (32 bytes from the OS CSPRNG), store it in the keychain item of section 2.3, write the `device` row for the current personal epoch.
- Unlock: read `device_key`, derive `device_wrap_key`, open the `device` row. On any failure (missing item, wrong `db_id`, failed unwrap) the app asks for the master password.
- Turning it off, or logging out with "delete local copy": delete the keychain item and the `device` row. A password change keeps the `device` row, because it does not depend on the password.
- A copy of `shellpp.db` on another machine has no keychain item, so it always needs the master password.

### 6.4 Refresh token

- `server_account.refresh_token` = wrap blob (n = 54, 95 bytes) of the UTF-8 token text (`shellpp_rt_` followed by 43 base64url characters).
- Key: `local_secrets_key` = HKDF(`vault_key[personal, secrets_epoch]`, `"shellpp/v1/local-secrets"` ‖ `device_id` ‖ `be32(secrets_epoch)`).
- AAD: `"shellpp/v1/wrap/refresh-token"` ‖ `device_id` ‖ `be16(len(o))` ‖ `o`, where `o` is the ASCII of `server_account.server_url`, the normalized origin that issued the token (the encoding of the auth keys, [security.md](security.md), section 5.5). The token therefore opens only for that origin: a `server_url` changed in the database never receives it, and the device signs in again. A token wrapped before run 9 (without the origin) no longer opens: such a device signs in once more. [decision] (D188)
- After a rotation of the personal vault, the token is rewrapped under the new epoch and `secrets_epoch` is updated.
- An offline recovery of the local copy on a bound device sets the column to `NULL`: the account is signed out on this device until the next sign-in or recovery through the server writes a new token ([sync-protocol.md](sync-protocol.md), section 5.2). (stage 0 decision)

### 6.5 KDF parameters

`local_auth.kdf` holds the KDF JSON; `local_auth.kdf_salt` holds 16 random bytes, new on every password change and email change, and at a registration that moves a personal vault bound to an earlier account (section 6.6); `local_auth.kdf_email` holds the normalized email the salt is bound to, or `NULL` on a device that has never been bound to an account (it stays after log out and account deletion, section 4.4).

```json
{"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4}
```

| Field | Meaning |
|---|---|
| `alg` | `argon2id` (Argon2 version 0x13) |
| `v` | KDF scheme version: `1` = Argon2id → 32-byte output → HKDF labels `shellpp/v1/*`. The Argon2id salt is `argon2_salt` (below) for the master password, whose output is `master_key`, and the file's own random `salt` for an encrypted export (section 13.3) |
| `m_kib`, `t`, `p` | memory in KiB, iterations, lanes. Desktop preset 65536 / 3 / 4, mobile preset 32768 / 4 / 2. |

The Argon2id salt is not `kdf_salt` itself but `argon2_salt` = SHA-256(`"shellpp/v1/kdf-salt"` ‖ `be16(len(e))` ‖ `e` ‖ `kdf_salt`), all 32 bytes, where `e` is the UTF-8 of `kdf_email` (empty when it is `NULL`) and `len(e)` its length in bytes (stage 0 decision).

The client refuses parameters outside the floor `alg = argon2id`, `v = 1`, `m_kib ≥ 32768`, `t ≥ 3`, `m_kib × t ≥ 131072` (the cost of the mobile preset), `1 ≤ p ≤ 16`, or above the ceiling of its platform: desktop `m_kib ≤ 1048576`, mobile `m_kib ≤ 262144`, both `t ≤ 16`. The same limits apply to encrypted exports (section 13.3). Details are in [security.md](security.md).

### 6.6 What changes when

| Event | Storage changes |
|---|---|
| First run | `meta` ids, `local_auth` (`kdf_email` `NULL`), personal `vaults` row (epoch 1), `kek` and `recovery` rows, records `vault_meta`, `recovery_key`, `device_info` |
| Registration | the `kek` row (keys re-derived with the account email) and `local_auth.kdf_email`. A personal vault bound to an earlier account is first moved (last row); registration then also writes a new `local_auth.kdf_salt`, a new `recovery_key` record in the new vault and the `recovery` row built from it ([sync-protocol.md](sync-protocol.md), section 4.3) |
| Sign-in on an empty device | `local_auth` from the account (`kdf`, `kdf_salt`, and as `kdf_email` the normalized email typed at sign-in), the `kek` row; the `recovery` row once the `recovery_key` record is pulled |
| Password change | `local_auth.kdf`, `kdf_salt`, `updated_at` and the `kek` row; `recovery` and `device` rows unchanged. With "my old password may be compromised": also a personal vault rotation |
| Email change | the same as a password change, plus `local_auth.kdf_email` and `server_account.email` |
| Recovery | the same as a password change, after opening the `recovery` row, together with a personal vault rotation; with an account also `local_auth.kdf_email` and `server_account.email`, set to the email accepted by `recover/begin` ([sync-protocol.md](sync-protocol.md), section 5.2) |
| Offline recovery of the local copy (bound device, server unreachable, refusing or rolled back) | the same as a local password change, after opening the `recovery` row: `local_auth.kdf`, `kdf_salt`, `updated_at` and the `kek` row; no rotation; `server_account.refresh_token` set to `NULL` ([sync-protocol.md](sync-protocol.md), section 5.2; stage 0 decision) |
| Recovery key reissue | a personal vault rotation with the `recovery` row of the new key, and the `recovery_key` record |
| New revision of the `recovery_key` record applied | the `recovery` row rebuilt from it |
| Personal vault rotation | new epoch: `kek`, `recovery`, `device` rows for it, a `chain` row, the refresh token rewrapped; records re-encrypted in the background |
| Team vault key arrives or rotates | `hpke` row for the new epoch, `chain` row, `vaults.current_epoch` |
| Sign-in on a device with local data | the account's personal vault replaces the local one: its rows and `local_auth` are written, the local records are re-encrypted into it with new record ids (D15), recordings are re-encrypted (section 8.4), the old personal vault's entries in `recently_deleted` are re-encrypted with the records under the new ids ([decision] (D93)), then the old personal vault's rows are deleted. A personal vault bound to an earlier account is kept instead (last row) |
| Sign-in to the account the personal vault was last bound to (rebind) | no merge, because the account's `personal_vault_id` equals the local one: `local_auth` and the `kek` row as at sign-in on an empty device, rows of any newer personal epoch (a lower epoch, or a held epoch whose server wrap opens to another key, changes nothing: section 6.2), `vaults.bound = 1`; records and record ids are kept, and the vault runs the full resync of [sync-protocol.md](sync-protocol.md), section 6.9 (stage 0 decision) |
| Binding to an account (registration, sign-in on an empty device or with local data, rebind) | also `server_account.server_generation` from the `GET /v1/server` of the connect wizard's first step, and `vaults.account_user_id` and `server_url` of every vault it binds ([sync-protocol.md](sync-protocol.md), section 3.4; [data-model.md](data-model.md), section 6.1). [decision] (D93) |
| Re-post of newer epochs after a server restore (personal or team vault) | none: the device sends wraps of the keys it holds ([sync-protocol.md](sync-protocol.md), sections 5.1 and 7.7). [decision] (D93) |
| Account deleted, here or from another device (`401 account_deleted`) | `server_account` row deleted; the personal vault `bound = 0`; team vaults `archived` with their key rows, `account_user_id` and `server_url`; the `meta` keys of the last account and `local_auth` kept, so the same password opens the local copy. [decision] (D93) for the other devices |
| A sync server's certificate trusted or changed by the user | the `server_tls` row of that origin written, replaced or ("Use the system's trust") deleted ([sync-protocol.md](sync-protocol.md), section 2.1). [decision] (D93) |
| Registration or sign-in to another account on a device whose personal vault was bound before | the local records are re-encrypted with new record ids into a new personal vault (at sign-in the account's, at registration a new `vault_id`, vault key, `kdf_salt` and `recovery_key` record), recordings are re-encrypted (section 8.4), and `local_auth.personal_vault_id` names the new vault. The old personal vault is not deleted: it becomes `archived` and keeps the earlier account's `user_keypair`, `ssh_id`, `member_pin`, `roster_entry` and `credential_binding` records, with its key rows kept as in section 6.2 ([data-model.md](data-model.md), section 18.5; stage 0 decision) |

The key changes of each event are one SQLite transaction; background re-encryption of records is not. Old database copies (pre-migration backups, user backups) keep the wraps they had, so they still open with the password of their time; after a personal vault rotation they open only data from before it. This is an accepted risk described in [security.md](security.md) (D11, RR6).

## 7. Command history

- Each command is one `history_entry` record (type `0x0601`, class `history` `0x06`) in the personal vault; per-host hint state is a `history_hint_state` record (`0x0602`). Fields are in [data-model.md](data-model.md).
- The server rejects the `history` class in team vaults, so history never leaves the personal vault.
- A command is stored only if history recording is on for the host (D20) and it passes the capture rules and the secret detector (D16, [security.md](security.md)). Commands that fail are never written, not even encrypted. Shell history imported from a server (UI 12.4) passes the same filters.
- History records are encrypted like every other record and decrypted lazily, when the History tab or the Shell History screen first needs them (D10).
- Deleting an entry writes a tombstone (or removes the row, section 4.6). "Clear history" for a host tombstones all of that host's entries in one transaction.
- Sync of the class is off by default (D15). There is no automatic expiry of history in the MVP (stage 0 decision).

## 8. Session recordings

(D16; layout chosen in stage 0)

### 8.1 File layout

`recordings/<file_id>.shpprec`, never synced.

| Offset | Size | Field |
|---|---|---|
| 0 | 7 | magic `shpprec` (ASCII) |
| 7 | 1 | version `0x01` |
| 8 | 16 | `file_id` (UUIDv7 bytes, equal to the file name) |
| 24 | 16 | personal `vault_id` |
| 40 | 4 | `key_epoch` (be32), epoch of the personal vault key |
| 44 | 19 | STREAM nonce prefix, random |
| 63 | … | encrypted chunks |

The header is bytes `[0, 63)`.

### 8.2 Chunk encryption

- Construction: STREAM with a big-endian 32-bit counter and a last-block flag (`StreamBE32` of the RustCrypto `aead` stream implementation) over XChaCha20-Poly1305.
- Key: `recording_key[file]` = HKDF-SHA256(`vault_key[personal, key_epoch]`, `"shellpp/v1/recording"` ‖ `file_id`, L = 32).
- Nonce of chunk `i` (24 bytes): nonce prefix (19) ‖ `be32(i)` (4) ‖ last flag (1: `0x01` for the final chunk, else `0x00`). `i` starts at 0.
- AAD of every chunk: header bytes `[0, 63)`.
- Every chunk except the final one has exactly 65 536 bytes of plaintext and is stored as 65 552 bytes (ciphertext ‖ 16-byte tag). The final chunk has 0 to 65 535 bytes of plaintext (16 to 65 551 bytes stored) and is always present in a complete file (stage 0 decision).
- Reading: frames of 65 552 bytes are non-final chunks; the remaining shorter frame is the final chunk. A file whose body length is a multiple of 65 552 has no final chunk: it was not closed (crash or power loss). Its complete chunks still decrypt and are shown with an "incomplete recording" notice.
- Writing: output is buffered in memory and a chunk is written when 64 KiB are buffered; closing the recording writes the final chunk. A crash loses at most the last 64 KiB.
- While the app is locked, sessions keep running (D9). An open recording keeps its own `recording_key[file]` in memory until it is closed, so output that arrives while locked is still recorded; this key opens only that one file (stage 0 decision, see [security.md](security.md)).

### 8.3 Plaintext

asciicast v2: newline-delimited JSON, UTF-8.

- First line: header `{"version": 2, "width": <columns>, "height": <rows>, "timestamp": <unix seconds>}`. No `env`, `title` or other fields, so the file holds no host data (stage 0 decision); the host and time are in the connection log entry.
- Then one line per output event: `[<seconds since start, float>, "o", "<output text>"]`. Only output events are recorded; input is never recorded.
- Size limit (run 5, D91): a recording holds at most 4 096 chunks (256 MiB of plaintext). The first output past it writes one marker event `[<seconds>, "m", "size limit"]`; later output of the session is not recorded, and the file closes as usual. Readers skip events other than `"o"`.

### 8.4 Lifecycle

(stage 0 decision)

- Recording starts with a session when the global toggle (UI 12.6, off by default) or the host override (D20) says so. The `connection_log_entry` record is written first and carries the `file_id`; then the file is created.
- The file is deleted only when this device writes or applies a tombstone of its log entry: the user deleted it, cleared the log, the entry expired (retention default 90 days, saved entries kept, D19) or another device deleted the synced entry. The entry is tombstoned first and the file deleted second; after unlock, a background scan deletes files whose log entry is present locally as a tombstone (left by a crash between the two steps).
- A missing log entry never deletes a file: recordings are never synced, so such a deletion could not be undone. A file whose log entry is absent locally (live or tombstone) is kept; the Logs screen lists it as a recording without a log entry, with view, export and delete actions.
- A log entry synced from another device may reference a `file_id` that does not exist locally; the log shows "recording not available on this device".
- Rotation of the personal vault does not touch recordings: `key_epoch` in the header selects the old key, which is reachable through the epoch chain.
- When the personal vault is replaced at sign-in on a device with local data, or moved to a new personal vault at registration (section 6.6), each recording is re-encrypted under the new personal vault key with the same `file_id` and a new nonce prefix, written to a new file and renamed over the old one. The old copy lives until the new one is written, and the re-encryption survives a failure or a crash ([decision] (D182)):
  - Before the transaction that replaces the vault, the client writes, for every recording sealed under the old vault and for every `.rekey` file that names the old vault, a file `recordings/<file_id>.<vault_id>.rekey` naming the new vault: bytes `[0, 7)` the magic `shpprky`, byte 7 the version `0x01`, `[8, 24)` the `file_id`, `[24, 40)` the new personal `vault_id`, `[40, 44)` `be32(e)` of its current epoch, then a 73-byte wrap blob (section 6.1) of the recording's current `recording_key[file]`, wrapped by HKDF(`recording_key[file]` of `vault_key[new, e]`, `"shellpp/v1/wrap/recording-rekey"`), with the AAD `"shellpp/v1/wrap/recording-rekey"` ‖ bytes `[0, 44)`. Each is written to a temporary name, synced and renamed. A failure here, a recording that cannot be read at that moment included, stops the sign-in before anything changed; only a file that is gone, too short or not a recording is left out. A merge given up later never deletes a `.rekey` file: it may be the only key of a recording of a store set aside by "Also delete the local copy".
  - After the commit, and at every unlock, each recording with a `.rekey` file for the current personal vault is re-encrypted chunk by chunk: every chunk opened with the old key is sealed again with the same plaintext and the same chunk boundaries, so the asciicast header, every event, the size-limit marker and a missing final chunk (an incomplete file) stay as they were, and one chunk at a time is in memory. A torn last fragment (a write cut by a full disk or a crash), which a reader leaves out (section 8.2), is left out too, so the file reads the same. The new file is written under the name `<file_id>.reencrypt-<16 hex digits>`, synced before the rename, and the `.rekey` file is deleted after it; on Unix the directory is synced after each rename. One re-encryption runs at a time, and a lock stops it. A file a session of this process is still writing is left for a later unlock; a file with a whole chunk that does not open keeps its old bytes and its `.rekey` file. Temporary files a crash left are deleted by the next run.
  - Until then the recording opens through its `.rekey` file. A `.rekey` file is deleted once its recording names the current personal vault or is gone; one for another vault (left by a sign-in that did not commit) stays until then.
- "Export to .txt" in the session log viewer (UI 11.2) writes the output with escape sequences stripped, as plain UTF-8 text, after a warning (D16).

## 9. Temporary files (SFTP Open)

(D16; layout and lifecycle are a stage 0 decision)

- Used only by Open and Open with… on a remote file (UI 5.4). Files opened from the local panel are opened in place.
- Location: the temporary directory of section 2.1. Each Open creates `tmp/<random UUID>/<file name>`, so the external program sees the original name and two files with the same name do not collide. Characters that the local file system does not allow (on Windows `< > : " / \ | ? *`, trailing dots and spaces, reserved names such as `CON`) are replaced with `_`.
- Permissions: directory `0700`, file `0600` on Unix.
- These files are plaintext by necessity. They exist only after an explicit user action, which shows a warning (D16). They are never synced, never put into backups and never written while the app is locked.
- While the SFTP session that opened a file is connected, changes to the file are uploaded back with a notification (UI 5.4).
- The file and its directory are deleted when tracking ends: when the SFTP tab or its connection closes, after a last check for unsaved changes. The whole `tmp/` directory is also wiped on app exit and on app start. A file that cannot be deleted because another program holds it open (Windows) is removed at the next start.

## 10. Writes and file integrity

- Database changes that belong together (a password change, a key epoch, a record and its bookkeeping) are one SQLite transaction.
- Files that replace other files (recording re-encryption, exports) are written to a new file in the same directory and renamed over the target, so a crash leaves either the old or the new version.
- Files are deleted by unlinking; the app does not overwrite them first, because overwriting is unreliable on SSDs and copy-on-write file systems. Database, recording and backup files hold only ciphertext; temporary files are plaintext, which is why section 9 keeps them short-lived.

## 11. Schema migrations

(stage 0 decision; SQL migrations are written in stage 1, D1)

- `PRAGMA user_version` is the schema version. `0` with no tables means a new database; the first run creates version 1 (section 4.2).
- Migrations ship inside the app as an ordered list of SQL scripts, one per version step N → N + 1. They run at start, before unlock: they never need keys and never touch envelope or wrap bytes.
- Before the first migration step, the app writes a consistent copy with `VACUUM INTO` to `backups/shellpp-<UTC yyyymmddThhmmssZ>.db.tmp`, syncs it and renames it to `backups/shellpp-<UTC yyyymmddThhmmssZ>.db` (section 10), then keeps only the 3 newest copies. Leftover `.db.tmp` files from an interrupted copy are deleted.
- Each step runs in its own transaction that also sets `PRAGMA user_version = N + 1`. A failed step rolls back and the app reports the error with the path of the backup.
- Migrations are additive: new tables, new columns with defaults, new indexes. They never rename or drop the tables and columns of section 4.2.
- If `user_version` is higher than the app knows, the database was written by a newer app: the app refuses to open it and does not modify it.
- Payload changes are not schema migrations; they follow section 5.6. Envelope changes would use a new envelope `version` byte; version 1 readers keep such rows unchanged.

## 12. Backup and restore

(stage 0 decision except where a decision is cited)

What a backup must contain:

- `shellpp.db` together with its `-wal` file when one exists (the WAL holds recent committed changes). The safe ways are to copy the data directory while the app is closed or to use the encrypted export (section 13.3).
- `recordings/` if recordings should be kept.
- Nothing else: `backups/`, `tmp/` and the logs are not needed, and the keychain item is optional (without it the master password unlocks).

Properties:

- A database copy is as protected as the original: every record is an envelope and every key is wrapped. It opens with the master password, or the recovery key, that was valid when the copy was taken.
- Pre-migration copies in `backups/` follow the same rules.

Restore:

1. Quit the app.
2. Replace `shellpp.db` with the copy and delete any `shellpp.db-wal` and `shellpp.db-shm` next to it. A WAL left over from another database must never be applied to the restored file.
3. Optionally restore `recordings/`.
4. Start the app. An older schema is migrated forward (section 11); a newer one is refused.

After a restore:

- A restored copy keeps its `device_id` and wrapped refresh token. Running it alongside the installation it came from is not supported: the shared refresh token is detected as reused and both are signed out ([sync-protocol.md](sync-protocol.md)). To set up a second device, sign in on a fresh installation or import an export.
- When the account password changed after the copy was taken, the copy unlocks with the old password and then needs a sign-in with the new one.

Removing the local copy (log out with "delete local copy", UI 12.2) deletes it: first the device key and the `device` row (when the OS keychain refuses, nothing changes), then `shellpp.db` with its side files, the side files first (when one cannot be deleted, nothing else is touched and the vault stays open, signed out), then every set-aside copy `shellpp.db.<yyyymmddThhmmssZ>[-failed][-<n>].old` with its side files, the backups `backups/shellpp-….db`, the recordings and their keys `recordings/<uuid>.…`, the temporary directory inside the data directory, and the device keys of this database and of the set-aside copies in the OS keychain (their `db_id` read from each copy's `meta`); `backups/` and `recordings/` go when nothing else is in them. The app then starts at first run with a new empty store; when none opens, the session closes and the next start begins at first run. Only these names go: a data directory the user chose (`SHELLPP_DATA_DIR`) keeps any other file, also inside `backups/` and `recordings/`, and the lock file and the logs (no secrets) stay; a temporary directory outside the data directory is emptied at the next start and exit as always. Recently deleted and the trust choices for sync servers are tables of the database and go with it ([decision] (D93)). Unsynced changes and the types that do not sync (history and the connection log unless turned on, anything turned off, and the recordings) are lost: the dialog warns, and asks a second time with the set-aside copies and backups listed and what only this device holds counted (UI 12.2). [decision] (D197, which revokes D190)

## 13. Export formats

### 13.1 Overview

| Export | Format id | Content | Default file name (stage 0 decision) |
|---|---|---|---|
| Plain JSON | `shellpp.export.v1` | all exportable records, secrets in clear text, after a warning (D16) | `shellpp-export-<UTC yyyymmddThhmmssZ>.json` |
| Encrypted JSON | `shellpp.export.encrypted.v1` | the plain export, encrypted with an export password | `shellpp-export-<UTC yyyymmddThhmmssZ>.encrypted.json` |
| OpenSSH client config | `ssh_config(5)` | SSH hosts as `Host` blocks | `shellpp-ssh-config` |
| OpenSSH keys | OpenSSH private and public key files | one key | `shellpp_<key alias>`, `.pub`, `-cert.pub` |
| known_hosts | `sshd(8)` known_hosts format | known host keys | `shellpp-known_hosts` |
| Terminal theme | `shellpp.terminal-theme.v1` | one theme | `<theme alias>.json` |

Exports are available at any time, also offline and for archived vaults (principles 3 and 4). The user chooses the vaults to include; the default is all. Default names never point at an existing OpenSSH file such as `~/.ssh/config`; writing over an existing file asks for confirmation.

### 13.2 Plain JSON export

```json
{
  "format": "shellpp.export.v1",
  "exported_at": "2026-10-18T22:31:05.123Z",
  "app_version": "0.1.0",
  "vaults": [
    {
      "vault_id": "0199f6a1-b2c0-7c11-8a3d-0e1f2a3b4c5d",
      "kind": "personal",
      "name": "Personal",
      "records": [
        {
          "record_id": "0199f6a1-b2c4-7a10-9b2e-5f6a7b8c9d0e",
          "revision": 3,
          "v": 1,
          "type": "host",
          "hlc": "0199f6a1b2c30001",
          "device_id": "0199f6a1-b2c3-7d4e-8f00-112233445566",
          "data": {}
        }
      ]
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `format` | always `shellpp.export.v1`; the JSON Schema has `$id` `urn:shellpp:schema:export:v1` (written in stage 3, D1) |
| `exported_at`, `app_version` | time of the export and the exporting app's version |
| `vaults[]` | one object per exported vault, personal first, then by `vault_id` |
| `vaults[].vault_id`, `kind` | as in the `vaults` table (`personal` or `team`) |
| `vaults[].name` | the vault name from its `vault_meta` record, for convenience |
| `vaults[].records[]` | the decrypted payload object of each record (section 5.5) plus `record_id` and `revision`, sorted by `record_id` |
| `settings` | the app settings by key ([data-model.md](data-model.md), section 16.1); optional (D34) |

- `data` of each type, and the references between records, are defined in [data-model.md](data-model.md); the export uses exactly those shapes.
- Included: live records of all types, conflict copies included. Excluded: tombstones, `user_keypair`, `recovery_key`, `member_pin` and `roster_entry`. Recordings are not embedded; a log entry keeps its `file_id`.
- Encoding: UTF-8 without a byte order mark, LF line endings, indented with 2 spaces.
- Versioning: within `v1`, new optional fields may appear at any level and readers ignore fields they do not know. Removing or changing the meaning of a field requires `shellpp.export.v2`; the app keeps reading every older format id. Payload versions inside records follow section 5.6.

### 13.3 Encrypted JSON export

```json
{
  "format": "shellpp.export.encrypted.v1",
  "kdf": {"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4},
  "salt": "<b64u, 16 bytes>",
  "nonce": "<b64u, 24 bytes>",
  "ciphertext": "<b64u>"
}
```

| Item | Value |
|---|---|
| Export password | chosen at export time, separate from the master password; NFC, UTF-8; at least 8 characters, like the master password (D29, D34) |
| `kdf` | the KDF JSON of section 6.5 with the preset of the exporting platform; a reader refuses values outside the bounds of section 6.5 |
| `salt` | 16 random bytes |
| Key | `export_key` = HKDF-SHA256(IKM = Argon2id(export password, `salt`, `m_kib`, `t`, `p`, output 32 bytes), salt = empty, info = `"shellpp/v1/export"`, L = 32) |
| AEAD | XChaCha20-Poly1305, single shot, `nonce` = 24 random bytes |
| AAD | `"shellpp.export.encrypted.v1"` (27 bytes) ‖ `salt` (16) ‖ `be32(m_kib)` ‖ `be32(t)` ‖ `be32(p)`, 55 bytes |
| Plaintext | the UTF-8 bytes of a complete plain export (section 13.2), no padding |
| `ciphertext` | ciphertext ‖ 16-byte tag |

Nothing about the content is in the clear: no time, version or vault list. Section 14.7 shows how to open the file without the app.

### 13.4 OpenSSH client config export

(stage 0 decision)

Hosts of the selected vaults whose protocol is SSH are written as `Host` blocks. Values are the effective values after group inheritance (D20); for a host in a team vault, the member's own `credential_binding` supplies the credentials (D13).

| Shell++ value | `ssh_config` keyword | Rule |
|---|---|---|
| host alias | `Host` | see below |
| address | `HostName` | always |
| port | `Port` | only when not 22 |
| username (host, identity or group) | `User` | when set |
| key (host or identity) | `IdentityFile` | `~/.ssh/shellpp_<key alias>` when the user also exports the keys (section 13.5); otherwise the line is written as a comment |
| certificate of that key | `CertificateFile` | `~/.ssh/shellpp_<key alias>-cert.pub`, same condition |
| jump host chain | `ProxyJump` | aliases of the jump hosts in order, comma-separated; a jump host that is not exported makes the line a comment |
| agent forwarding | `ForwardAgent yes` | when on |
| keepalive | `ServerAliveInterval` | when set on the host or a group; `0` means off |
| environment variables | `SetEnv NAME=value` | values containing spaces are double-quoted; a value containing `"` or a line break is written as a comment |
| forwarding rules of the host with "start on connection" | `LocalForward`, `RemoteForward`, `DynamicForward` | bind address included when it is not `127.0.0.1`; rules without autostart are written as comments |
| HTTP or SOCKS proxy | none | written as a comment: OpenSSH has no direct option for it |
| password, passphrase | none | never written |
| group path | none | a `# Group: a / b` comment above the block |
| label, notes, tags, color, theme, encoding, startup snippet, other protocols | none | not written; the JSON export keeps them |

Aliases:

1. Take the host label, or the address when the label is empty; normalize to NFC and lowercase.
2. Replace every character outside `[a-z0-9._-]` with `-`, collapse repeated `-`, trim `-` and `.` at both ends.
3. If the result is empty, use `host-` followed by the last 8 hex digits of the `record_id`.
4. If the alias is already taken in this file, append `-2`, `-3`, … in the order (group path, label, `record_id`).

Key aliases use the same rule on the key label. Hosts that are not SSH (local terminal, serial, Telnet only) are skipped and counted in the summary. The file starts with comments naming the app, its version and the export time, uses LF line endings and is UTF-8.

```text
# Exported by Shell++ 0.1.0 at 2026-10-18T22:31:05.123Z
# Group: prod / web
Host web-1
  HostName 203.0.113.10
  User deploy
  IdentityFile ~/.ssh/shellpp_deploy-key
  ProxyJump bastion
  ServerAliveInterval 30

Host bastion
  HostName bastion.example.com
  Port 2222
  User admin
```

### 13.5 OpenSSH key export

| File | Content | Unix mode |
|---|---|---|
| `shellpp_<key alias>` | private key in the OpenSSH format (`-----BEGIN OPENSSH PRIVATE KEY-----`), encrypted with a passphrase if the user sets one (offered by UI 7.3), using the standard OpenSSH key encryption of the `ssh-key` crate | `0600` |
| `shellpp_<key alias>.pub` | one line: algorithm, base64 public key, comment = key label (`shellpp:<email>` for the SSH ID, D13) | `0644` |
| `shellpp_<key alias>-cert.pub` | the certificate, when the key has one | `0644` |

- The private key file is readable by the user only before a byte of it is written, and an existing file is replaced by a new one (a temporary file renamed over it): mode `0600` and no symbolic link followed on Unix; on Windows a protected DACL with one entry, full control for the current user, and no inherited entries (D42). The plain Export Data file is written the same way.
- Key aliases are unique over the three file names, so no key's file has the name of another key's file (the labels `work` and `work.pub` give `shellpp_work` and `shellpp_work.pub-2`).
- Keys imported from PEM or PuTTY `.ppk` are written in the OpenSSH format.
- FIDO2 keys (`sk-ssh-ed25519`, `sk-ecdsa`) are written in the OpenSSH security-key format, which holds the key handle, not a secret.
- A passphrase saved in the vault is never written to a file.

### 13.6 known_hosts export

One line per `known_host` record, sorted by host pattern and key type, LF line endings:

| Record | Line |
|---|---|
| plain entry, port 22 | `<host> <key type> <base64 key>` |
| plain entry, other port | `[<host>]:<port> <key type> <base64 key>` |
| hashed entry | `\|1\|<salt>\|<hash> <key type> <base64 key>`, the pattern exactly as stored (D21) |
| revoked key | the same line prefixed with `@revoked` |

No comments are written. `@cert-authority` entries are never stored (D21), so they are never exported.

### 13.7 Terminal theme JSON

(D25; schema id chosen in stage 0)

The format is defined normatively in [design-system.md](design-system.md), section 7.2; this section summarizes it. A theme file is a UTF-8 JSON object; its JSON Schema has `$id` `urn:shellpp:schema:terminal-theme:v1`. The built-in themes are examples: `apps/desktop/src/theme/terminal/graphite.json` and `paper.json`. Imported themes are stored as `terminal_theme` records with the same object (D25).

| Field | Type | Required |
|---|---|---|
| `name` | string, 1–64 characters | yes |
| `version` | integer, `1` | yes |
| `background`, `foreground`, `cursor`, `cursorText` | `#RRGGBB` | yes |
| `selection` | `#RRGGBB` or `#RRGGBBAA`, drawn over `background` | yes |
| `selectionForeground` | `#RRGGBB` | no |
| `searchMatch`, `searchMatchActive` | `#RRGGBB` | yes |
| `ansi` | object with the 16 ANSI colors, each `#RRGGBB`: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `brightBlack`, `brightRed`, `brightGreen`, `brightYellow`, `brightBlue`, `brightMagenta`, `brightCyan`, `brightWhite` | yes |

Hex digits may be upper or lower case. Version 1 allows no other fields: a file with an unknown field or another `version` is rejected. `.itermcolors` files and Windows Terminal schemes are converted into this format on import (D25).

### 13.8 Import idempotency

Running an import again does not create duplicates (SPEC, "Import"). The matching rules are owned by [data-model.md](data-model.md); this table lists the keys that the exports above produce, so that a re-import of Shell++'s own export is idempotent.

| Source | Idempotency key (D21) |
|---|---|
| Native JSON, plain or encrypted | the exported `record_id` |
| `~/.ssh/config` | source type and `Host` alias; the export writes stable, unique aliases (section 13.4) |
| PuTTY sessions | session name |
| CSV (`label, address, port, username, group, tags`) | (address, port, username) |
| known_hosts, key files | see [data-model.md](data-model.md) |

Existing records are never overwritten by an import (D21).

## 14. Decrypt your data manually

This section is enough to read a Shell++ database, a recording or an encrypted export with standard libraries. Work on a copy: quit the app, then copy `shellpp.db` and, if present, `shellpp.db-wal` into an empty folder.

### 14.1 Parameters

| Step | Algorithm and parameters |
|---|---|
| Password encoding | Unicode NFC, then UTF-8; no trimming |
| Argon2id salt | SHA-256(`"shellpp/v1/kdf-salt"` ‖ `be16(len(e))` ‖ `e` ‖ `local_auth.kdf_salt`), 32 bytes, where `e` is the UTF-8 of `local_auth.kdf_email` (empty when `NULL`) |
| KDF | Argon2id, version 0x13; salt = the Argon2id salt above; memory = `m_kib` KiB, iterations = `t`, lanes = `p` from `local_auth.kdf`; output 32 bytes; no secret, no associated data → `master_key` |
| HKDF | HKDF-SHA256 (RFC 5869), empty salt (equivalent to 32 zero bytes), output 32 bytes, `info` = label ‖ binary context |
| AEAD | XChaCha20-Poly1305 (the IETF variant with a 24-byte nonce, as in libsodium's `crypto_aead_xchacha20poly1305_ietf`), 16-byte tag after the ciphertext |
| KEK | HKDF(`master_key`, `"shellpp/v1/kek"`) |
| Recovery KEK | HKDF(`recovery_key`, `"shellpp/v1/recovery-kek"`) |
| Personal vault key | open `key_wraps` row (`personal_vault_id`, `current_epoch`, `kek`) with the KEK: nonce = bytes `[1, 25)`, ciphertext = bytes `[25, 73)`, AAD = `"shellpp/v1/wrap/kek"` ‖ `vault_id` ‖ `be32(epoch)` |
| Older epochs | for `e` from `current_epoch` down to 2: `chain_key` = HKDF(`vault_key[e]`, `"shellpp/v1/epoch-chain"` ‖ `vault_id` ‖ `be32(e)`); open the `chain` row with epoch `e`, AAD = `"shellpp/v1/wrap/epoch-chain"` ‖ `vault_id` ‖ `be32(e − 1)` → `vault_key[e − 1]` |
| Record key | HKDF(`vault_key[key_epoch]`, `"shellpp/v1/record"` ‖ `vault_id` ‖ `be32(key_epoch)`) |
| Record | nonce = envelope `[48, 72)`, ciphertext = envelope `[72, end)`, AAD = `"shellpp/v1/record"` ‖ envelope `[0, 48)`; then unpad (section 5.3) and parse JSON |

```text
kdf_email, kdf_salt ─SHA-256 "shellpp/v1/kdf-salt"─► argon2_salt
password ─NFC, UTF-8─► Argon2id(argon2_salt, m_kib, t, p) ─► master_key
master_key ─HKDF "shellpp/v1/kek"─► kek
kek + key_wraps(personal, e, 'kek') ─XChaCha20-Poly1305─► vault_key[personal, e]
vault_key[v, e] + key_wraps(v, e, 'chain') ─► vault_key[v, e − 1] ─► …
vault_key[v, e] ─HKDF "shellpp/v1/record" ‖ vault_id ‖ be32(e)─► record_key[v, e]
record_key + envelope ─XChaCha20-Poly1305─► padded JSON ─unpad─► payload
```

### 14.2 Steps for the personal vault

1. Open the copy with any SQLite 3 client.
2. Read `personal_vault_id`, `kdf`, `kdf_salt` and `kdf_email` from `local_auth`, and `current_epoch` from `vaults` for that vault.
3. Derive `master_key` and `kek` from the master password (section 14.1). Without the password, use the recovery key (section 14.4) instead.
4. Open the `kek` row of the current epoch to get the vault key, then walk the `chain` rows to get every older epoch key.
5. For each `records` row of the vault: read `key_epoch` from envelope bytes `[44, 48)`, derive the record key, decrypt, unpad and parse. The result is the payload of section 5.5; `data` is described per type in [data-model.md](data-model.md).

### 14.3 Reference script

Python 3 with `argon2-cffi` and `PyNaCl` (`pip install argon2-cffi pynacl`). It prints one decrypted payload per line for the personal vault. Run it as `python3 shellpp_read.py <copy of shellpp.db>`, or with `--recovery` to unlock with the recovery key. It reads secrets from the terminal, never from the command line.

```python
import getpass
import hashlib
import hmac
import json
import sqlite3
import struct
import sys
import unicodedata

from argon2.low_level import Type, hash_secret_raw
from nacl.bindings import crypto_aead_xchacha20poly1305_ietf_decrypt as aead_open

CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def be32(x: int) -> bytes:
    return struct.pack(">I", x)


def hkdf32(ikm: bytes, info: bytes) -> bytes:
    # HKDF-SHA256 with an empty salt (same as 32 zero bytes) and a 32-byte output.
    prk = hmac.new(b"\x00" * 32, ikm, hashlib.sha256).digest()
    return hmac.new(prk, info + b"\x01", hashlib.sha256).digest()


def open_wrap(blob: bytes, key: bytes, aad: bytes) -> bytes:
    if blob[0] != 0x01:
        raise ValueError("unknown wrap blob version")
    return aead_open(blob[25:], aad, blob[1:25], key)


def unpad(data: bytes) -> bytes:
    body = data.rstrip(b"\x00")
    if not body or body[-1] != 0x80:
        raise ValueError("bad padding")
    return body[:-1]


def decode_recovery_key(text: str) -> bytes:
    s = text.upper().replace("-", "").replace(" ", "")
    s = s.translate(str.maketrans("ILO", "110"))
    if len(s) == 58 and s.startswith("RK1"):
        s = s[3:]
    if len(s) != 55:
        raise ValueError("expected 55 characters after RK1-")
    n = 0
    for ch in s:
        n = n * 32 + CROCKFORD.index(ch)
    raw = (n >> 3).to_bytes(34, "big")  # 275 bits, the last 3 are zero
    key, check = raw[:32], raw[32:]
    if hashlib.sha256(key).digest()[:2] != check:
        raise ValueError("recovery key checksum mismatch")
    return key


def main() -> None:
    db = sqlite3.connect(sys.argv[1])
    vault_id, kdf_text, salt, kdf_email = db.execute(
        "SELECT personal_vault_id, kdf, kdf_salt, kdf_email FROM local_auth").fetchone()
    (epoch,) = db.execute(
        "SELECT current_epoch FROM vaults WHERE vault_id = ?", (vault_id,)).fetchone()

    def wrap(kind: str, e: int) -> bytes:
        (blob,) = db.execute(
            "SELECT wrap FROM key_wraps WHERE vault_id = ? AND epoch = ? AND kind = ?",
            (vault_id, e, kind)).fetchone()
        return blob

    if "--recovery" in sys.argv:
        recovery_key = decode_recovery_key(getpass.getpass("Recovery key: "))
        key = hkdf32(recovery_key, b"shellpp/v1/recovery-kek")
        vault_key = open_wrap(wrap("recovery", epoch), key,
                              b"shellpp/v1/wrap/recovery" + vault_id + be32(epoch))
    else:
        kdf = json.loads(kdf_text)
        if kdf["alg"] != "argon2id" or kdf["v"] != 1:
            raise SystemExit("unsupported KDF")
        password = unicodedata.normalize("NFC", getpass.getpass("Master password: "))
        email = (kdf_email or "").encode("utf-8")  # empty if never bound to an account
        argon2_salt = hashlib.sha256(
            b"shellpp/v1/kdf-salt" + struct.pack(">H", len(email)) + email + salt).digest()
        master_key = hash_secret_raw(
            password.encode("utf-8"), argon2_salt, time_cost=kdf["t"],
            memory_cost=kdf["m_kib"], parallelism=kdf["p"], hash_len=32, type=Type.ID,
            version=19)
        kek = hkdf32(master_key, b"shellpp/v1/kek")
        vault_key = open_wrap(wrap("kek", epoch), kek,
                              b"shellpp/v1/wrap/kek" + vault_id + be32(epoch))

    keys = {epoch: vault_key}
    for e in range(epoch, 1, -1):  # the key of epoch e opens the key of epoch e - 1
        chain_key = hkdf32(keys[e], b"shellpp/v1/epoch-chain" + vault_id + be32(e))
        keys[e - 1] = open_wrap(wrap("chain", e), chain_key,
                                b"shellpp/v1/wrap/epoch-chain" + vault_id + be32(e - 1))

    rows = db.execute("SELECT envelope FROM records WHERE vault_id = ?", (vault_id,))
    for (env,) in rows:
        if env[0] != 0x01:
            raise ValueError("unknown envelope version")
        key_epoch = struct.unpack(">I", env[44:48])[0]
        record_key = hkdf32(keys[key_epoch],
                            b"shellpp/v1/record" + vault_id + be32(key_epoch))
        plain = aead_open(env[72:], b"shellpp/v1/record" + env[:48], env[48:72], record_key)
        print(json.dumps(json.loads(unpad(plain)), ensure_ascii=False))


if __name__ == "__main__":
    main()
```

Test vectors for every construction in this section are added with the stage 1 implementation (stage 0 decision).

### 14.4 With the recovery key instead of the password

1. Decode the recovery key: drop the `RK1-` prefix, spaces and hyphens; ignore case; map `I` and `L` to `1` and `O` to `0`; decode the 55 characters as Crockford base32 (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, most significant bit first) into 34 bytes, dropping the 3 trailing zero bits. Bytes `[0, 32)` are the key; bytes `[32, 34)` must equal the first 2 bytes of SHA-256 of the key. The format is owned by [security.md](security.md).
2. `recovery_kek` = HKDF(`recovery_key`, `"shellpp/v1/recovery-kek"`).
3. Open the `recovery` row of the current epoch with AAD `"shellpp/v1/wrap/recovery"` ‖ `vault_id` ‖ `be32(epoch)`, then continue with step 4 of section 14.2.

### 14.5 Team vaults

Team vault keys are wrapped with HPKE (RFC 9180); any implementation that supports the Auth mode works.

1. Decrypt the personal vault of the account the team vault was bound to (step 2 names it): the current one (section 14.2), or, when `vaults.account_user_id` names an earlier account, the archived personal vault with the same `account_user_id`, whose `kek` row opens with the same KEK (section 6.2; [decision] (D93)). Take the X25519 private key from its `user_keypair` record (fields in [data-model.md](data-model.md)).
2. Find your own `user_id` and server: the team vault's own `vaults.account_user_id` and `vaults.server_url` ([decision] (D93)); for a row without them (written before that migration), `server_account.user_id` and `server_url`, or the `meta` keys `account_user_id` and `account_server_url` when no account is bound any more.
3. Read the `hpke` row of the team vault for its `current_epoch`: `sender_user_id`, `hpke_enc` (`enc`) and `wrap` (`ct`).
4. Find the sender's X25519 public key: your own public key when `sender_user_id` is your own id (the vault creator wraps epoch 1 to itself), otherwise the `member_pin` record with that server and `user_id` in the personal vault of step 1.
5. Open the wrap: mode `mode_auth` (`0x02`); KEM DHKEM(X25519, HKDF-SHA256) `0x0020`; KDF HKDF-SHA256 `0x0001`; AEAD ChaCha20Poly1305 `0x0003`; no PSK; `aad` empty; `info` = `"shellpp/v1/hpke/vault-key"` (25 bytes) ‖ `vault_id` (16) ‖ `be32(epoch)` ‖ recipient `user_id` (16, your own) ‖ `sender_user_id` (16), 77 bytes. The plaintext is `vault_key[v, epoch]` (32 bytes).
6. Walk the `chain` rows and decrypt the records exactly as for the personal vault.

### 14.6 Recordings

Using the helpers of section 14.3 and `keys` of the personal vault:

```python
FRAME = 65536 + 16


def read_recording(path: str, keys: dict, key: bytes | None = None) -> bytes:
    with open(path, "rb") as f:
        data = f.read()
    header, body = data[:63], data[63:]
    if header[:7] != b"shpprec" or header[7] != 0x01:
        raise ValueError("not a version 1 recording")
    file_id, prefix = header[8:24], header[44:63]
    key_epoch = struct.unpack(">I", header[40:44])[0]
    if key is None:  # else the key of its `.rekey` file (below)
        key = hkdf32(keys[key_epoch], b"shellpp/v1/recording" + file_id)
    out, i = bytearray(), 0
    while len(body) >= FRAME:  # full frames are never the final chunk
        out += aead_open(body[:FRAME], header, prefix + be32(i) + b"\x00", key)
        body, i = body[FRAME:], i + 1
    if not body:
        raise ValueError("incomplete recording: no final chunk")
    out += aead_open(body, header, prefix + be32(i) + b"\x01", key)
    return bytes(out)  # asciicast v2 text, section 8.3
```

A recording that still waits to be re-encrypted after a sign-in merged this device's data into an account (section 8.4) is sealed under the key of the earlier vault, which the account's `keys` do not hold. It opens through its `recordings/<file_id>.<vault_id>.rekey` file, whose `vault_id` is the account's personal vault and whose epoch is one of `keys`:

```python
def rekey_key(path: str, keys: dict) -> bytes:
    with open(path, "rb") as f:
        data = f.read()
    head, blob = data[:44], data[44:]
    if head[:7] != b"shpprky" or head[7] != 0x01:
        raise ValueError("not a version 1 .rekey file")
    file_id, epoch = head[8:24], struct.unpack(">I", head[40:44])[0]
    recording_key = hkdf32(keys[epoch], b"shellpp/v1/recording" + file_id)
    label = b"shellpp/v1/wrap/recording-rekey"
    return open_wrap(blob, hkdf32(recording_key, label), label + head)


# read_recording(recording_path, keys, rekey_key(rekey_path, keys))
```

### 14.7 Encrypted export

With the imports and helpers of section 14.3:

```python
import base64


def b64u(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def open_export(doc: dict, password: str) -> dict:
    kdf = doc["kdf"]
    if doc["format"] != "shellpp.export.encrypted.v1" or kdf["alg"] != "argon2id":
        raise ValueError("unsupported file")
    salt, nonce, ct = b64u(doc["salt"]), b64u(doc["nonce"]), b64u(doc["ciphertext"])
    secret = hash_secret_raw(
        unicodedata.normalize("NFC", password).encode("utf-8"), salt,
        time_cost=kdf["t"], memory_cost=kdf["m_kib"], parallelism=kdf["p"],
        hash_len=32, type=Type.ID, version=19)
    key = hkdf32(secret, b"shellpp/v1/export")
    aad = (b"shellpp.export.encrypted.v1" + salt
           + be32(kdf["m_kib"]) + be32(kdf["t"]) + be32(kdf["p"]))
    return json.loads(aead_open(ct, aad, nonce, key))  # a shellpp.export.v1 document
```
