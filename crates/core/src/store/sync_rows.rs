//! The sync tables of schema 2 (docs/storage-format.md, section 4.2; D93):
//! the bound account, the per-vault sync state, Recently deleted, the
//! records a resync brought back, duplicate pairs and the trust choices for
//! sync servers. Reads on [`Store`], writes on [`WriteTx`].

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{
    RECORD_COLUMNS, RawRecord, RecordRow, Store, StoreError, WriteTx, raw_record, record_row,
};

/// The `server_account` row (zero or one).
#[derive(Clone, PartialEq, Eq)]
pub struct ServerAccountRow {
    pub server_url: String,
    pub user_id: Uuid,
    pub email: String,
    /// Wrap blob of the refresh token under `local_secrets_key`; `None`
    /// while signed out (an offline recovery, a refused refresh).
    pub refresh_token: Option<Vec<u8>>,
    pub secrets_epoch: u32,
    pub bound_at: String,
    pub server_generation: Option<Vec<u8>>,
}

impl std::fmt::Debug for ServerAccountRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerAccountRow")
            .field("user_id", &self.user_id)
            .field("signed_in", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

/// A `sync_state` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStateRow {
    pub vault_id: Uuid,
    pub cursor: u64,
    pub last_sync_at: Option<String>,
    /// An error code only.
    pub last_error: Option<String>,
    /// Classes awaiting a class-limited full resync.
    pub resync_classes: Vec<u8>,
}

/// Where a Recently deleted entry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletedOrigin {
    Local,
    Sync,
}

/// A `recently_deleted` row: the last live envelope of a deleted record.
#[derive(Clone, PartialEq, Eq)]
pub struct RecentlyDeletedRow {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub class: u8,
    pub deleted_at: String,
    pub origin: DeletedOrigin,
    pub envelope: Vec<u8>,
}

impl std::fmt::Debug for RecentlyDeletedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecentlyDeletedRow")
            .field("record_id", &self.record_id)
            .field("class", &self.class)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// A `duplicate_pairs` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicatePairRow {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub other_record_id: Uuid,
    pub found_at: String,
    pub kept: bool,
}

/// How a sync server's certificate is trusted beyond the system roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsTrustKind {
    Pinned,
    Ca,
}

/// A `server_tls` row (plaintext: needed while locked; public data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsTrustRow {
    pub server_url: String,
    pub kind: TlsTrustKind,
    /// SHA-256 of the server certificate (`Pinned`) or of the CA (`Ca`), DER.
    pub cert_sha256: [u8; 32],
    pub ca_der: Option<Vec<u8>>,
    pub created_at: String,
}

fn uuid_of(bytes: Vec<u8>) -> Result<Uuid, StoreError> {
    Uuid::from_slice(&bytes).map_err(|_| StoreError::Corrupt("uuid column"))
}

fn classes_text(classes: &[u8]) -> Option<String> {
    (!classes.is_empty())
        .then(|| serde_json::to_string(classes).unwrap_or_else(|_| "[]".to_owned()))
}

fn classes_of(text: Option<String>) -> Vec<u8> {
    text.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn origin_str(origin: DeletedOrigin) -> &'static str {
    match origin {
        DeletedOrigin::Local => "local",
        DeletedOrigin::Sync => "sync",
    }
}

type RawServerAccount = (
    String,
    Vec<u8>,
    String,
    Option<Vec<u8>>,
    i64,
    String,
    Option<Vec<u8>>,
);

fn server_account_of(raw: RawServerAccount) -> Result<ServerAccountRow, StoreError> {
    Ok(ServerAccountRow {
        server_url: raw.0,
        user_id: uuid_of(raw.1)?,
        email: raw.2,
        refresh_token: raw.3,
        secrets_epoch: u32::try_from(raw.4).map_err(|_| StoreError::Corrupt("secrets_epoch"))?,
        bound_at: raw.5,
        server_generation: raw.6,
    })
}

type RawSyncState = (i64, Option<String>, Option<String>, Option<String>);
type RawDeleted = (Vec<u8>, Vec<u8>, i64, String, String, Vec<u8>);
type RawPair = (Vec<u8>, Vec<u8>, Vec<u8>, String, bool);
type RawTls = (String, Vec<u8>, Option<Vec<u8>>, String);

impl Store {
    /// The bound account, if any.
    pub fn server_account(&self) -> Result<Option<ServerAccountRow>, StoreError> {
        let raw: Option<RawServerAccount> = self
            .conn
            .query_row(
                "SELECT server_url, user_id, email, refresh_token, secrets_epoch, bound_at,
                 server_generation FROM server_account WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        raw.map(server_account_of).transpose()
    }

    /// The sync state of a vault (a default row when there is none).
    pub fn sync_state(&self, vault_id: &Uuid) -> Result<SyncStateRow, StoreError> {
        let raw: Option<RawSyncState> = self
            .conn
            .query_row(
                "SELECT cursor, last_sync_at, last_error, resync_classes FROM sync_state
                 WHERE vault_id = ?1",
                params![&vault_id.as_bytes()[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(match raw {
            Some((cursor, last_sync_at, last_error, classes)) => SyncStateRow {
                vault_id: *vault_id,
                cursor: cursor.max(0) as u64,
                last_sync_at,
                last_error,
                resync_classes: classes_of(classes),
            },
            None => SyncStateRow {
                vault_id: *vault_id,
                ..SyncStateRow::default()
            },
        })
    }

    /// The account and server a vault was last bound to (D93).
    pub fn vault_binding(
        &self,
        vault_id: &Uuid,
    ) -> Result<(Option<Uuid>, Option<String>), StoreError> {
        let raw: Option<(Option<Vec<u8>>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT account_user_id, server_url FROM vaults WHERE vault_id = ?1",
                params![&vault_id.as_bytes()[..]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((user, url)) = raw else {
            return Ok((None, None));
        };
        Ok((user.map(uuid_of).transpose()?, url))
    }

    /// The pending rows of a vault (`revision <> synced_revision`).
    pub fn records_pending(&self, vault_id: &Uuid) -> Result<Vec<RecordRow>, StoreError> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {RECORD_COLUMNS} FROM records WHERE vault_id = ?1 AND revision <> synced_revision
             ORDER BY record_id"
        ))?;
        let raws: Vec<RawRecord> = stmt
            .query_map(params![&vault_id.as_bytes()[..]], raw_record)?
            .collect::<Result<_, _>>()?;
        raws.into_iter().map(record_row).collect()
    }

    /// A record row by id in any vault (a pulled id must belong to one vault).
    pub fn record_anywhere(&self, record_id: &Uuid) -> Result<Option<RecordRow>, StoreError> {
        let raw = self
            .conn
            .prepare_cached(&format!(
                "SELECT {RECORD_COLUMNS} FROM records WHERE record_id = ?1"
            ))?
            .query_row(params![&record_id.as_bytes()[..]], raw_record)
            .optional()?;
        raw.map(record_row).transpose()
    }

    /// Recently deleted, newest first.
    pub fn recently_deleted(&self) -> Result<Vec<RecentlyDeletedRow>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT vault_id, record_id, class, deleted_at, origin, envelope FROM recently_deleted
             ORDER BY deleted_at DESC, record_id",
        )?;
        let raws: Vec<RawDeleted> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<Result<_, _>>()?;
        raws.into_iter()
            .map(|raw| {
                Ok(RecentlyDeletedRow {
                    vault_id: uuid_of(raw.0)?,
                    record_id: uuid_of(raw.1)?,
                    class: u8::try_from(raw.2).map_err(|_| StoreError::Corrupt("class"))?,
                    deleted_at: raw.3,
                    origin: if raw.4 == "sync" {
                        DeletedOrigin::Sync
                    } else {
                        DeletedOrigin::Local
                    },
                    envelope: raw.5,
                })
            })
            .collect()
    }

    /// The records a resync brought back: (vault, record, when).
    pub fn resync_returned(&self) -> Result<Vec<(Uuid, Uuid, String)>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT vault_id, record_id, returned_at FROM resync_returned ORDER BY returned_at",
        )?;
        let raws: Vec<(Vec<u8>, Vec<u8>, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        raws.into_iter()
            .map(|raw| Ok((uuid_of(raw.0)?, uuid_of(raw.1)?, raw.2)))
            .collect()
    }

    /// The duplicate pairs.
    pub fn duplicate_pairs(&self) -> Result<Vec<DuplicatePairRow>, StoreError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT vault_id, record_id, other_record_id, found_at, kept FROM duplicate_pairs
             ORDER BY found_at",
        )?;
        let raws: Vec<RawPair> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<_, _>>()?;
        raws.into_iter()
            .map(|raw| {
                Ok(DuplicatePairRow {
                    vault_id: uuid_of(raw.0)?,
                    record_id: uuid_of(raw.1)?,
                    other_record_id: uuid_of(raw.2)?,
                    found_at: raw.3,
                    kept: raw.4,
                })
            })
            .collect()
    }

    /// The trust choice for a server origin.
    pub fn server_tls(&self, server_url: &str) -> Result<Option<TlsTrustRow>, StoreError> {
        let raw: Option<RawTls> = self
            .conn
            .query_row(
                "SELECT kind, cert_sha256, ca_der, created_at FROM server_tls WHERE server_url = ?1",
                params![server_url],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((kind, hash, ca_der, created_at)) = raw else {
            return Ok(None);
        };
        Ok(Some(TlsTrustRow {
            server_url: server_url.to_owned(),
            kind: if kind == "ca" {
                TlsTrustKind::Ca
            } else {
                TlsTrustKind::Pinned
            },
            cert_sha256: hash
                .try_into()
                .map_err(|_| StoreError::Corrupt("cert_sha256"))?,
            ca_der,
            created_at,
        }))
    }
}

impl WriteTx<'_> {
    pub fn server_account_put(&mut self, row: &ServerAccountRow) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO server_account (id, server_url, user_id, email, refresh_token,
             secrets_epoch, bound_at, server_generation) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET server_url = excluded.server_url,
               user_id = excluded.user_id, email = excluded.email,
               refresh_token = excluded.refresh_token, secrets_epoch = excluded.secrets_epoch,
               bound_at = excluded.bound_at, server_generation = excluded.server_generation",
            params![
                row.server_url,
                &row.user_id.as_bytes()[..],
                row.email,
                row.refresh_token,
                i64::from(row.secrets_epoch),
                row.bound_at,
                row.server_generation,
            ],
        )?;
        Ok(())
    }

    pub fn server_account_delete(&mut self) -> Result<(), StoreError> {
        self.tx.execute("DELETE FROM server_account", [])?;
        Ok(())
    }

    pub fn server_account(&self) -> Result<Option<ServerAccountRow>, StoreError> {
        let raw: Option<RawServerAccount> = self
            .tx
            .query_row(
                "SELECT server_url, user_id, email, refresh_token, secrets_epoch, bound_at,
                 server_generation FROM server_account WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        raw.map(server_account_of).transpose()
    }

    pub fn sync_state_put(&mut self, row: &SyncStateRow) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO sync_state (vault_id, cursor, last_sync_at, last_error, resync_classes)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (vault_id) DO UPDATE SET cursor = excluded.cursor,
               last_sync_at = excluded.last_sync_at, last_error = excluded.last_error,
               resync_classes = excluded.resync_classes",
            params![
                &row.vault_id.as_bytes()[..],
                row.cursor as i64,
                row.last_sync_at,
                row.last_error,
                classes_text(&row.resync_classes),
            ],
        )?;
        Ok(())
    }

    pub fn sync_state_delete(&mut self, vault_id: &Uuid) -> Result<(), StoreError> {
        self.tx.execute(
            "DELETE FROM sync_state WHERE vault_id = ?1",
            params![&vault_id.as_bytes()[..]],
        )?;
        Ok(())
    }

    /// Sets (or clears) the account and server a vault is bound to.
    pub fn vault_binding_set(
        &mut self,
        vault_id: &Uuid,
        account_user_id: Option<&Uuid>,
        server_url: Option<&str>,
    ) -> Result<(), StoreError> {
        self.tx.execute(
            "UPDATE vaults SET account_user_id = ?2, server_url = ?3 WHERE vault_id = ?1",
            params![
                &vault_id.as_bytes()[..],
                account_user_id.map(|id| id.as_bytes().to_vec()),
                server_url,
            ],
        )?;
        Ok(())
    }

    pub fn recently_deleted_put(&mut self, row: &RecentlyDeletedRow) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO recently_deleted (vault_id, record_id, class, deleted_at, origin, envelope)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (vault_id, record_id) DO UPDATE SET class = excluded.class,
               deleted_at = excluded.deleted_at, origin = excluded.origin,
               envelope = excluded.envelope",
            params![
                &row.vault_id.as_bytes()[..],
                &row.record_id.as_bytes()[..],
                i64::from(row.class),
                row.deleted_at,
                origin_str(row.origin),
                row.envelope,
            ],
        )?;
        Ok(())
    }

    pub fn recently_deleted_delete(
        &mut self,
        vault_id: &Uuid,
        record_id: &Uuid,
    ) -> Result<(), StoreError> {
        self.tx.execute(
            "DELETE FROM recently_deleted WHERE vault_id = ?1 AND record_id = ?2",
            params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..]],
        )?;
        Ok(())
    }

    /// Removes the entries deleted before `cutoff` (RFC 3339 text); returns how many.
    pub fn recently_deleted_purge(&mut self, cutoff: &str) -> Result<usize, StoreError> {
        Ok(self.tx.execute(
            "DELETE FROM recently_deleted WHERE deleted_at < ?1",
            params![cutoff],
        )?)
    }

    pub fn recently_deleted_clear(&mut self) -> Result<usize, StoreError> {
        Ok(self.tx.execute("DELETE FROM recently_deleted", [])?)
    }

    pub fn resync_returned_put(
        &mut self,
        vault_id: &Uuid,
        record_id: &Uuid,
        at: &str,
    ) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO resync_returned (vault_id, record_id, returned_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (vault_id, record_id) DO UPDATE SET returned_at = excluded.returned_at",
            params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..], at],
        )?;
        Ok(())
    }

    pub fn resync_returned_delete(
        &mut self,
        vault_id: &Uuid,
        record_id: &Uuid,
    ) -> Result<(), StoreError> {
        self.tx.execute(
            "DELETE FROM resync_returned WHERE vault_id = ?1 AND record_id = ?2",
            params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..]],
        )?;
        Ok(())
    }

    pub fn resync_returned_purge(&mut self, cutoff: &str) -> Result<usize, StoreError> {
        Ok(self.tx.execute(
            "DELETE FROM resync_returned WHERE returned_at < ?1",
            params![cutoff],
        )?)
    }

    pub fn duplicate_pair_put(&mut self, row: &DuplicatePairRow) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO duplicate_pairs (vault_id, record_id, other_record_id, found_at, kept)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (vault_id, record_id, other_record_id) DO UPDATE SET kept = excluded.kept",
            params![
                &row.vault_id.as_bytes()[..],
                &row.record_id.as_bytes()[..],
                &row.other_record_id.as_bytes()[..],
                row.found_at,
                row.kept,
            ],
        )?;
        Ok(())
    }

    /// Removes every pair that names `record_id` (it is no longer live).
    pub fn duplicate_pairs_forget(
        &mut self,
        vault_id: &Uuid,
        record_id: &Uuid,
    ) -> Result<(), StoreError> {
        self.tx.execute(
            "DELETE FROM duplicate_pairs WHERE vault_id = ?1 AND (record_id = ?2 OR other_record_id = ?2)",
            params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..]],
        )?;
        Ok(())
    }

    pub fn server_tls_put(&mut self, row: &TlsTrustRow) -> Result<(), StoreError> {
        self.tx.execute(
            "INSERT INTO server_tls (server_url, kind, cert_sha256, ca_der, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (server_url) DO UPDATE SET kind = excluded.kind,
               cert_sha256 = excluded.cert_sha256, ca_der = excluded.ca_der,
               created_at = excluded.created_at",
            params![
                row.server_url,
                match row.kind {
                    TlsTrustKind::Pinned => "pinned",
                    TlsTrustKind::Ca => "ca",
                },
                &row.cert_sha256[..],
                row.ca_der,
                row.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn server_tls_delete(&mut self, server_url: &str) -> Result<(), StoreError> {
        self.tx.execute(
            "DELETE FROM server_tls WHERE server_url = ?1",
            params![server_url],
        )?;
        Ok(())
    }

    /// Deletes a vault with every row that belongs to it: records, key rows,
    /// sync state, Recently deleted, brought-back and duplicate entries (the
    /// old personal vault after a merge at sign-in, data-model 18.5).
    pub fn vault_purge(&mut self, vault_id: &Uuid) -> Result<(), StoreError> {
        let id = &vault_id.as_bytes()[..];
        for table in [
            "records",
            "key_wraps",
            "sync_state",
            "recently_deleted",
            "resync_returned",
            "duplicate_pairs",
            "vaults",
        ] {
            self.tx.execute(
                &format!("DELETE FROM {table} WHERE vault_id = ?1"),
                params![id],
            )?;
        }
        Ok(())
    }
}
