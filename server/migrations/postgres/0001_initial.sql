-- The server schema, version 1 (docs/sync-protocol.md, section 14), PostgreSQL.
-- The team vault tables (invitations, members, HPKE wraps) are created now,
-- although their endpoints come with stage 8 (run 7 plan, rule 1.7.3).

CREATE TABLE users (
  user_id                UUID PRIMARY KEY,
  email                  TEXT NOT NULL UNIQUE,
  email_verified         BOOLEAN NOT NULL DEFAULT FALSE,
  auth_verifier          TEXT NOT NULL,
  recovery_verifier      TEXT NOT NULL,
  kdf                    JSONB NOT NULL,
  kdf_salt               BYTEA NOT NULL,
  personal_vault_id      UUID NOT NULL,
  kek_wrap               BYTEA NOT NULL,
  recovery_wrap          BYTEA NOT NULL,
  x25519_public_key      BYTEA NOT NULL,
  ssh_id_public_key      TEXT NOT NULL,
  totp_secret_enc        BYTEA,
  totp_enabled           BOOLEAN NOT NULL DEFAULT FALSE,
  totp_last_step         BIGINT,
  disabled               BOOLEAN NOT NULL DEFAULT FALSE,
  quota_max_records      BIGINT,
  quota_max_bytes        BIGINT,
  created_at             TIMESTAMPTZ NOT NULL,
  credentials_changed_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE devices (
  device_id    UUID PRIMARY KEY,
  user_id      UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  created_at   TIMESTAMPTZ NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL,
  revoked_at   TIMESTAMPTZ
);

CREATE TABLE refresh_tokens (
  token_hash     BYTEA PRIMARY KEY,
  family_id      UUID NOT NULL,
  device_id      UUID NOT NULL REFERENCES devices ON DELETE CASCADE,
  user_id        UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  created_at     TIMESTAMPTZ NOT NULL,
  expires_at     TIMESTAMPTZ NOT NULL,
  used_at        TIMESTAMPTZ,
  revoked_at     TIMESTAMPTZ,
  successor_hash BYTEA,
  retried_at     TIMESTAMPTZ
);

CREATE TABLE deleted_account_tokens (
  token_hash BYTEA PRIMARY KEY,
  deleted_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE server_state (
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  server_generation BYTEA NOT NULL,
  changed_at        TIMESTAMPTZ NOT NULL
);

CREATE TABLE access_tokens (
  token_hash BYTEA PRIMARY KEY,
  family_id  UUID NOT NULL,
  device_id  UUID NOT NULL REFERENCES devices ON DELETE CASCADE,
  user_id    UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE vaults (
  vault_id         UUID PRIMARY KEY,
  kind             TEXT NOT NULL,
  current_epoch    INTEGER NOT NULL,
  last_seq         BIGINT NOT NULL DEFAULT 0,
  min_valid_seq    BIGINT NOT NULL DEFAULT 0,
  rotation_pending BOOLEAN NOT NULL DEFAULT FALSE,
  created_by       UUID NOT NULL,
  created_at       TIMESTAMPTZ NOT NULL
);

CREATE TABLE vault_members (
  vault_id UUID NOT NULL REFERENCES vaults ON DELETE CASCADE,
  user_id  UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  role     TEXT NOT NULL,
  added_by UUID NOT NULL,
  added_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (vault_id, user_id)
);

CREATE TABLE vault_key_epochs (
  vault_id      UUID NOT NULL REFERENCES vaults ON DELETE CASCADE,
  epoch         INTEGER NOT NULL,
  prev_key_wrap BYTEA,
  created_by    UUID NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (vault_id, epoch)
);

CREATE TABLE vault_key_wraps (
  vault_id       UUID NOT NULL REFERENCES vaults ON DELETE CASCADE,
  epoch          INTEGER NOT NULL,
  user_id        UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  sender_user_id UUID NOT NULL,
  hpke_enc       BYTEA NOT NULL,
  hpke_ct        BYTEA NOT NULL,
  created_at     TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (vault_id, epoch, user_id)
);

CREATE TABLE records (
  vault_id          UUID NOT NULL REFERENCES vaults ON DELETE CASCADE,
  record_id         UUID NOT NULL,
  seq               BIGINT NOT NULL,
  class             SMALLINT NOT NULL,
  revision          BIGINT NOT NULL,
  key_epoch         INTEGER NOT NULL,
  tombstone         BOOLEAN NOT NULL,
  envelope          BYTEA NOT NULL,
  created_by        UUID NOT NULL,
  updated_by        UUID NOT NULL,
  updated_by_device UUID NOT NULL,
  received_at       TIMESTAMPTZ NOT NULL,
  tombstoned_at     TIMESTAMPTZ,
  PRIMARY KEY (vault_id, record_id),
  UNIQUE (vault_id, seq)
);

CREATE TABLE invitations (
  invitation_id     UUID PRIMARY KEY,
  token_hash        BYTEA NOT NULL UNIQUE,
  vault_id          UUID NOT NULL REFERENCES vaults ON DELETE CASCADE,
  created_by        UUID NOT NULL,
  role              TEXT NOT NULL,
  email             TEXT,
  status            TEXT NOT NULL,
  invitee_user_id   UUID,
  invitee_x25519_pk BYTEA,
  created_at        TIMESTAMPTZ NOT NULL,
  expires_at        TIMESTAMPTZ NOT NULL,
  accepted_at       TIMESTAMPTZ,
  confirmed_at      TIMESTAMPTZ
);

CREATE TABLE registration_codes (
  code_hash  BYTEA PRIMARY KEY,
  note       TEXT,
  uses_left  INTEGER NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE auth_tickets (
  ticket_hash BYTEA PRIMARY KEY,
  kind        TEXT NOT NULL,
  user_id     UUID NOT NULL REFERENCES users ON DELETE CASCADE,
  device_id   UUID NOT NULL,
  attempts    INTEGER NOT NULL DEFAULT 0,
  expires_at  TIMESTAMPTZ NOT NULL
);

CREATE TABLE email_codes (
  user_id    UUID PRIMARY KEY REFERENCES users ON DELETE CASCADE,
  new_email  TEXT,
  code_hash  BYTEA NOT NULL,
  attempts   INTEGER NOT NULL DEFAULT 0,
  expires_at TIMESTAMPTZ NOT NULL
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
