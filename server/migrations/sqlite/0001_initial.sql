-- The server schema, version 1 (docs/sync-protocol.md, section 14), SQLite:
-- UUIDs are 16-byte BLOBs, times RFC 3339 UTC text with milliseconds,
-- booleans 0 or 1, JSON text (src/db.rs).
-- The team vault tables (invitations, members, HPKE wraps) are created now,
-- although their endpoints come with stage 8 (run 7 plan, rule 1.7.3).

CREATE TABLE users (
  user_id                BLOB PRIMARY KEY,
  email                  TEXT NOT NULL UNIQUE,
  email_verified         INTEGER NOT NULL DEFAULT 0,
  auth_verifier          TEXT NOT NULL,
  recovery_verifier      TEXT NOT NULL,
  kdf                    TEXT NOT NULL,
  kdf_salt               BLOB NOT NULL,
  personal_vault_id      BLOB NOT NULL,
  kek_wrap               BLOB NOT NULL,
  recovery_wrap          BLOB NOT NULL,
  x25519_public_key      BLOB NOT NULL,
  ssh_id_public_key      TEXT NOT NULL,
  totp_secret_enc        BLOB,
  totp_enabled           INTEGER NOT NULL DEFAULT 0,
  totp_last_step         INTEGER,
  disabled               INTEGER NOT NULL DEFAULT 0,
  quota_max_records      INTEGER,
  quota_max_bytes        INTEGER,
  created_at             TEXT NOT NULL,
  credentials_changed_at TEXT NOT NULL
);

CREATE TABLE devices (
  device_id    BLOB PRIMARY KEY,
  user_id      BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  created_at   TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  revoked_at   TEXT
);

CREATE TABLE refresh_tokens (
  token_hash     BLOB PRIMARY KEY,
  family_id      BLOB NOT NULL,
  device_id      BLOB NOT NULL REFERENCES devices ON DELETE CASCADE,
  user_id        BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  created_at     TEXT NOT NULL,
  expires_at     TEXT NOT NULL,
  used_at        TEXT,
  revoked_at     TEXT,
  successor_hash BLOB,
  retried_at     TEXT
);

CREATE TABLE deleted_account_tokens (
  token_hash BLOB PRIMARY KEY,
  deleted_at TEXT NOT NULL
);

CREATE TABLE server_state (
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  server_generation BLOB NOT NULL,
  changed_at        TEXT NOT NULL
);

CREATE TABLE access_tokens (
  token_hash BLOB PRIMARY KEY,
  family_id  BLOB NOT NULL,
  device_id  BLOB NOT NULL REFERENCES devices ON DELETE CASCADE,
  user_id    BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);

CREATE TABLE vaults (
  vault_id         BLOB PRIMARY KEY,
  kind             TEXT NOT NULL,
  current_epoch    INTEGER NOT NULL,
  last_seq         INTEGER NOT NULL DEFAULT 0,
  min_valid_seq    INTEGER NOT NULL DEFAULT 0,
  rotation_pending INTEGER NOT NULL DEFAULT 0,
  created_by       BLOB NOT NULL,
  created_at       TEXT NOT NULL
);

CREATE TABLE vault_members (
  vault_id BLOB NOT NULL REFERENCES vaults ON DELETE CASCADE,
  user_id  BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  role     TEXT NOT NULL,
  added_by BLOB NOT NULL,
  added_at TEXT NOT NULL,
  PRIMARY KEY (vault_id, user_id)
);

CREATE TABLE vault_key_epochs (
  vault_id      BLOB NOT NULL REFERENCES vaults ON DELETE CASCADE,
  epoch         INTEGER NOT NULL,
  prev_key_wrap BLOB,
  created_by    BLOB NOT NULL,
  created_at    TEXT NOT NULL,
  PRIMARY KEY (vault_id, epoch)
);

CREATE TABLE vault_key_wraps (
  vault_id       BLOB NOT NULL REFERENCES vaults ON DELETE CASCADE,
  epoch          INTEGER NOT NULL,
  user_id        BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  sender_user_id BLOB NOT NULL,
  hpke_enc       BLOB NOT NULL,
  hpke_ct        BLOB NOT NULL,
  created_at     TEXT NOT NULL,
  PRIMARY KEY (vault_id, epoch, user_id)
);

CREATE TABLE records (
  vault_id          BLOB NOT NULL REFERENCES vaults ON DELETE CASCADE,
  record_id         BLOB NOT NULL,
  seq               INTEGER NOT NULL,
  class             INTEGER NOT NULL,
  revision          INTEGER NOT NULL,
  key_epoch         INTEGER NOT NULL,
  tombstone         INTEGER NOT NULL,
  envelope          BLOB NOT NULL,
  created_by        BLOB NOT NULL,
  updated_by        BLOB NOT NULL,
  updated_by_device BLOB NOT NULL,
  received_at       TEXT NOT NULL,
  tombstoned_at     TEXT,
  PRIMARY KEY (vault_id, record_id),
  UNIQUE (vault_id, seq)
);

CREATE TABLE invitations (
  invitation_id     BLOB PRIMARY KEY,
  token_hash        BLOB NOT NULL UNIQUE,
  vault_id          BLOB NOT NULL REFERENCES vaults ON DELETE CASCADE,
  created_by        BLOB NOT NULL,
  role              TEXT NOT NULL,
  email             TEXT,
  status            TEXT NOT NULL,
  invitee_user_id   BLOB,
  invitee_x25519_pk BLOB,
  created_at        TEXT NOT NULL,
  expires_at        TEXT NOT NULL,
  accepted_at       TEXT,
  confirmed_at      TEXT
);

CREATE TABLE registration_codes (
  code_hash  BLOB PRIMARY KEY,
  note       TEXT,
  uses_left  INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE auth_tickets (
  ticket_hash BLOB PRIMARY KEY,
  kind        TEXT NOT NULL,
  user_id     BLOB NOT NULL REFERENCES users ON DELETE CASCADE,
  device_id   BLOB NOT NULL,
  attempts    INTEGER NOT NULL DEFAULT 0,
  expires_at  TEXT NOT NULL
);

CREATE TABLE email_codes (
  user_id    BLOB PRIMARY KEY REFERENCES users ON DELETE CASCADE,
  new_email  TEXT,
  code_hash  BLOB NOT NULL,
  attempts   INTEGER NOT NULL DEFAULT 0,
  expires_at TEXT NOT NULL
);

CREATE UNIQUE INDEX records_record_id ON records (record_id);
CREATE INDEX records_tombstoned ON records (tombstoned_at) WHERE tombstone;
CREATE INDEX records_created_by ON records (created_by) WHERE NOT tombstone;
CREATE INDEX refresh_tokens_family ON refresh_tokens (family_id);
CREATE INDEX refresh_tokens_device ON refresh_tokens (device_id);
CREATE INDEX access_tokens_family ON access_tokens (family_id);
CREATE INDEX access_tokens_device ON access_tokens (device_id);
CREATE INDEX vault_members_user ON vault_members (user_id);
CREATE INDEX invitations_vault ON invitations (vault_id);
