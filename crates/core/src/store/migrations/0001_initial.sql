-- Shell++ local database, schema version 1 (docs/storage-format.md, section 4.2).
-- Applied to a new, empty database in one transaction that also sets
-- PRAGMA user_version = 1. Later migrations may add columns, indexes and
-- tables but never rename or drop what is defined here.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE local_auth (
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  personal_vault_id BLOB NOT NULL,
  kdf               TEXT NOT NULL,
  kdf_salt          BLOB NOT NULL,
  kdf_email         TEXT,
  updated_at        TEXT NOT NULL
);

CREATE TABLE vaults (
  vault_id      BLOB PRIMARY KEY,
  kind          TEXT NOT NULL CHECK (kind IN ('personal', 'team')),
  status        TEXT NOT NULL CHECK (status IN ('active', 'archived')),
  role          TEXT CHECK (role IN ('owner', 'editor', 'viewer')),
  current_epoch INTEGER NOT NULL,
  bound         INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL
);

CREATE TABLE key_wraps (
  vault_id       BLOB NOT NULL REFERENCES vaults (vault_id),
  epoch          INTEGER NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('kek', 'recovery', 'device', 'hpke', 'chain')),
  sender_user_id BLOB,
  hpke_enc       BLOB,
  wrap           BLOB NOT NULL,
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
  tombstoned_at   TEXT,
  server_seq      INTEGER,
  envelope        BLOB NOT NULL,
  base_envelope   BLOB,
  PRIMARY KEY (vault_id, record_id)
);
CREATE UNIQUE INDEX records_record_id ON records (record_id);
CREATE INDEX records_by_class ON records (vault_id, class);
CREATE INDEX records_pending ON records (vault_id) WHERE revision <> synced_revision;

CREATE TABLE sync_state (
  vault_id       BLOB PRIMARY KEY REFERENCES vaults (vault_id),
  cursor         INTEGER NOT NULL DEFAULT 0,
  last_sync_at   TEXT,
  last_error     TEXT,
  resync_classes TEXT
);

CREATE TABLE server_account (
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  server_url    TEXT NOT NULL,
  user_id       BLOB NOT NULL,
  email         TEXT NOT NULL,
  refresh_token BLOB,
  secrets_epoch INTEGER NOT NULL,
  bound_at      TEXT NOT NULL
);

CREATE TABLE local_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
