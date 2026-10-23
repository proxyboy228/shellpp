-- Shell++ local database, schema version 2 (docs/storage-format.md, section 4.2,
-- "Added with stage 7"; D93): the account a vault was bound to, the server
-- generation, Recently deleted, the records a resync brought back, duplicate
-- pairs from sync and the trust choices for sync servers. Additive only.

ALTER TABLE vaults ADD COLUMN account_user_id BLOB;
ALTER TABLE vaults ADD COLUMN server_url TEXT;
ALTER TABLE server_account ADD COLUMN server_generation BLOB;

CREATE TABLE recently_deleted (
  vault_id   BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id  BLOB NOT NULL,
  class      INTEGER NOT NULL,
  deleted_at TEXT NOT NULL,
  origin     TEXT NOT NULL CHECK (origin IN ('local', 'sync')),
  envelope   BLOB NOT NULL,
  PRIMARY KEY (vault_id, record_id)
);
CREATE INDEX recently_deleted_by_time ON recently_deleted (deleted_at);

CREATE TABLE resync_returned (
  vault_id    BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id   BLOB NOT NULL,
  returned_at TEXT NOT NULL,
  PRIMARY KEY (vault_id, record_id)
);

CREATE TABLE duplicate_pairs (
  vault_id        BLOB NOT NULL REFERENCES vaults (vault_id),
  record_id       BLOB NOT NULL,
  other_record_id BLOB NOT NULL,
  found_at        TEXT NOT NULL,
  kept            INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (vault_id, record_id, other_record_id)
);

CREATE TABLE server_tls (
  server_url  TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('pinned', 'ca')),
  cert_sha256 BLOB NOT NULL,
  ca_der      BLOB,
  created_at  TEXT NOT NULL
);
