//! SQLite storage: connection settings, schema version 1, migrations with a
//! backup first, and typed row access (docs/storage-format.md, sections 4, 10
//! and 11).
//!
//! One connection is owned by the store; writes are serialized by the caller.
//! Every group of changes that belongs together goes through [`Store::write`],
//! one `BEGIN IMMEDIATE` transaction that is rolled back on error.
//!
//! Owner: W1. Public API: see the stage 1 plan, section "store".

mod dir_lock;
mod migrate;
mod rows;
mod sync_rows;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use shellpp_sync_api::envelope as wire;
use uuid::Uuid;

pub use rows::{KeyWrapRow, LocalAuthRow, RecordRow, VaultRow, WrapKind};
pub use sync_rows::{
    DeletedOrigin, DuplicatePairRow, RecentlyDeletedRow, ServerAccountRow, SyncStateRow,
    TlsTrustKind, TlsTrustRow,
};

pub(crate) use dir_lock::DirLock;
use migrate::MigrationPlan;
use rows::{
    parse_role, parse_vault_kind, parse_vault_status, role_str, u32_from_db, u64_from_db,
    u64_to_db, uuid_from_blob, vault_kind_str, vault_status_str,
};

/// The schema version created by this app (`PRAGMA user_version`).
pub const SCHEMA_VERSION: u32 = 2;

/// The SQL of schema version 1 (docs/storage-format.md, section 4.2).
pub const SCHEMA_V1_SQL: &str = include_str!("migrations/0001_initial.sql");

/// `meta` key: UUIDv7 text naming this database (keychain account, device wrap context).
pub const META_DB_ID: &str = "db_id";
/// `meta` key: UUIDv7 text of this device.
pub const META_DEVICE_ID: &str = "device_id";
/// `meta` key: timestamp of the first run.
pub const META_CREATED_AT: &str = "created_at";
/// `meta` key: semver of the last app version that opened the database.
pub const META_LAST_APP_VERSION: &str = "last_app_version";
/// `meta` key: `user_id` of the last bound account (stage 7).
pub const META_ACCOUNT_USER_ID: &str = "account_user_id";
/// `meta` key: server origin of the last bound account (stage 7).
pub const META_ACCOUNT_SERVER_URL: &str = "account_server_url";

/// SQLite busy timeout.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

const PLAN: MigrationPlan = MigrationPlan {
    v1_sql: SCHEMA_V1_SQL,
    steps: migrate::MIGRATIONS,
};

const _: () = assert!(SCHEMA_VERSION as usize == 1 + migrate::MIGRATIONS.len());

/// Errors of the storage layer. Messages never contain paths, secrets or record data.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database schema is newer than this app; nothing was modified.
    #[error("database schema {found} is newer than supported")]
    NewerSchema { found: u32 },
    /// A migration step failed and was rolled back; a backup was written first.
    #[error("migration to schema {to} failed")]
    Migration { to: u32 },
    /// SQLite reported an error.
    #[error("sqlite error")]
    Sqlite(#[from] rusqlite::Error),
    /// File system error (creating the data directory, backups).
    #[error("i/o error")]
    Io(#[from] std::io::Error),
    /// A row is inconsistent (for example envelope header and columns disagree).
    #[error("corrupt row: {0}")]
    Corrupt(&'static str),
    /// Another app process has the data directory open.
    #[error("data directory is in use by another process")]
    InUse,
}

impl StoreError {
    /// Stable error code (docs/architecture.md, section 3.4).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NewerSchema { .. } => "newer_schema",
            Self::Io(_) => "io_error",
            Self::Corrupt(_) => "damaged",
            Self::Migration { .. } | Self::Sqlite(_) | Self::InUse => "storage_error",
        }
    }
}

/// What [`Store::open`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A new database was created at the current schema version.
    Created,
    /// An existing database at the current schema version was opened.
    Opened,
    /// An older database was migrated after writing `backup`.
    Migrated { from: u32, backup: PathBuf },
}

/// The local database.
pub struct Store {
    conn: Connection,
    data_dir: Option<PathBuf>,
    /// The exclusive lock of the data directory; `None` in memory. Declared
    /// after `conn`, so the connection closes before the lock is released.
    dir_lock: Option<DirLock>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No path: diagnostic output never contains file paths.
        f.debug_struct("Store")
            .field("in_memory", &self.data_dir.is_none())
            .finish()
    }
}

/// Unix: directory mode 0700. Elsewhere the per-user profile ACLs apply.
pub(crate) fn restrict_dir(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Unix: file mode 0600. Elsewhere the per-user profile ACLs apply.
pub(crate) fn restrict_file(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Creates an empty file that only the owner can read (unix 0600) so SQLite
/// never creates the database with wider permissions.
fn create_private_file(path: &Path) -> Result<(), StoreError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Connection settings of docs/storage-format.md, section 4.1.
/// `journal_size_limit = 0` truncates the WAL whenever a checkpoint resets
/// it, so old page versions do not linger in `shellpp.db-wal`.
fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA secure_delete = ON;
         PRAGMA synchronous = FULL;
         PRAGMA journal_size_limit = 0;",
    )?;
    Ok(())
}

/// Reads the schema version without opening the file for writing, so a
/// database of a newer app is never touched (no WAL checkpoint, no header
/// change). `None` when the file cannot be read this way.
fn peek_user_version(path: &Path) -> Option<u32> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    migrate::user_version(&conn).ok()
}

impl Store {
    /// Opens or creates `shellpp.db` in `data_dir`.
    ///
    /// Creates the data directory if missing (unix 0700); takes the exclusive
    /// lock of the directory ([`StoreError::InUse`] when another process holds
    /// it after a few seconds of waiting); creates the database file (unix
    /// 0600); applies the connection settings; `user_version` 0
    /// with no tables creates schema 1 without a backup; an older version is
    /// backed up with `VACUUM INTO 'backups/shellpp-<UTC yyyymmddThhmmssZ>.db'`
    /// and migrated step by step, keeping the 3 newest backups; a newer
    /// version returns [`StoreError::NewerSchema`] without modifying the
    /// file. Then writes `meta.last_app_version = app_version`.
    pub fn open(data_dir: &Path, app_version: &str) -> Result<(Store, OpenOutcome), StoreError> {
        Self::open_with(data_dir, app_version, &PLAN, SystemTime::now())
    }

    pub(crate) fn open_with(
        data_dir: &Path,
        app_version: &str,
        plan: &MigrationPlan,
        now: SystemTime,
    ) -> Result<(Store, OpenOutcome), StoreError> {
        let target = plan.target();
        fs::create_dir_all(data_dir)?;
        restrict_dir(data_dir)?;
        // Before anything reads or writes the database.
        let dir_lock = DirLock::acquire(data_dir, dir_lock::LOCK_WAIT)?;
        let path = data_dir.join(crate::ids::DB_FILE_NAME);

        let existed = path.is_file();
        if existed {
            if let Some(found) = peek_user_version(&path)
                && found > target
            {
                return Err(StoreError::NewerSchema { found });
            }
        } else {
            create_private_file(&path)?;
        }

        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Checked again before any pragma that writes to the file.
        let version = migrate::user_version(&conn)?;
        if version > target {
            return Err(StoreError::NewerSchema { found: version });
        }
        // A file with tables but no schema version was not written by this
        // app: refuse it before switching it to WAL mode.
        if version == 0 && migrate::schema_object_count(&conn)? != 0 {
            return Err(StoreError::Corrupt(
                "database has tables but no schema version",
            ));
        }
        apply_pragmas(&conn)?;
        restrict_file(&path)?;

        let outcome = if version == 0 {
            migrate::create(&mut conn, plan)?;
            OpenOutcome::Created
        } else if version == target {
            OpenOutcome::Opened
        } else {
            let backup = migrate::backup(&conn, data_dir, now)?;
            migrate::run_steps(&mut conn, plan, version)?;
            OpenOutcome::Migrated {
                from: version,
                backup,
            }
        };

        let mut store = Store {
            conn,
            data_dir: Some(data_dir.to_path_buf()),
            dir_lock: Some(dir_lock),
        };
        store.write(|tx| tx.meta_set(META_LAST_APP_VERSION, app_version))?;
        Ok((store, outcome))
    }

    /// An in-memory database at the current schema version (tests).
    pub fn open_in_memory() -> Result<Store, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        apply_pragmas(&conn)?;
        migrate::create(&mut conn, &PLAN)?;
        Ok(Store {
            conn,
            data_dir: None,
            dir_lock: None,
        })
    }

    /// `PRAGMA user_version`.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        migrate::user_version(&self.conn)
    }

    /// The data directory, `None` for an in-memory store.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// Another handle on the lock of the data directory (`None` in memory).
    /// While it lives, other processes stay locked out even after this store
    /// is dropped, and a [`Store::open`] of the same directory in this
    /// process shares it ("Start with a new vault" closes the database and
    /// creates a new one under it).
    pub(crate) fn dir_lock(&self) -> Option<DirLock> {
        self.dir_lock.clone()
    }

    /// Path of the database file (tests: the storage canary scans it and the WAL).
    pub fn db_path(&self) -> Option<PathBuf> {
        self.data_dir
            .as_ref()
            .map(|dir| dir.join(crate::ids::DB_FILE_NAME))
    }

    fn reader(&self) -> Reader<'_> {
        Reader(&self.conn)
    }

    /// A `meta` value.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.reader().meta_get(key)
    }

    /// The `local_auth` row, `None` before the first run.
    pub fn local_auth(&self) -> Result<Option<LocalAuthRow>, StoreError> {
        self.reader().local_auth()
    }

    /// One vault.
    pub fn vault(&self, vault_id: &Uuid) -> Result<Option<VaultRow>, StoreError> {
        self.reader().vault(vault_id)
    }

    /// Every vault, ordered by id.
    pub fn vaults(&self) -> Result<Vec<VaultRow>, StoreError> {
        self.reader().vaults()
    }

    /// One key wrap.
    pub fn key_wrap(
        &self,
        vault_id: &Uuid,
        epoch: u32,
        kind: WrapKind,
    ) -> Result<Option<KeyWrapRow>, StoreError> {
        self.reader().key_wrap(vault_id, epoch, kind)
    }

    /// Every key wrap of a vault, ordered by epoch and kind.
    pub fn key_wraps(&self, vault_id: &Uuid) -> Result<Vec<KeyWrapRow>, StoreError> {
        self.reader().key_wraps(vault_id)
    }

    /// One record row.
    pub fn record(
        &self,
        vault_id: &Uuid,
        record_id: &Uuid,
    ) -> Result<Option<RecordRow>, StoreError> {
        self.reader().record(vault_id, record_id)
    }

    /// All rows of the vault (tombstones included), optionally limited to
    /// classes, ordered by record id.
    pub fn records(
        &self,
        vault_id: &Uuid,
        classes: Option<&[u8]>,
    ) -> Result<Vec<RecordRow>, StoreError> {
        self.reader().records(vault_id, classes)
    }

    /// All rows of the vault for loading the index at unlock, ordered by
    /// record id. Unlike [`Store::records`], a row whose columns cannot be
    /// converted (wrong SQLite type, value out of range, id not 16 bytes) does
    /// not fail the whole read: it comes back as an [`UnreadableRow`], so it
    /// can be left out of the index and reported as damaged while every other
    /// record opens (docs/storage-format.md, section 4.6).
    pub fn records_for_load(
        &self,
        vault_id: &Uuid,
    ) -> Result<Vec<Result<RecordRow, UnreadableRow>>, StoreError> {
        self.reader().records_for_load(vault_id)
    }

    /// A `local_settings` value (JSON text).
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.reader().setting(key)
    }

    /// Every `local_settings` row, ordered by key.
    pub fn settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        self.reader().settings()
    }

    /// `PRAGMA wal_checkpoint(TRUNCATE)`: copies every WAL frame into the
    /// database file and truncates `shellpp.db-wal` to zero bytes, so older
    /// versions of changed or deleted rows (replaced key wraps) do not stay in
    /// the WAL (docs/storage-format.md, section 4.1). `Ok(false)` when SQLite
    /// could not complete it because another connection was busy. A no-op for
    /// an in-memory store.
    pub fn checkpoint_truncate(&self) -> Result<bool, StoreError> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        Ok(busy == 0)
    }

    /// Runs `f` in one transaction (`BEGIN IMMEDIATE`); commits when it
    /// returns `Ok`, rolls back on `Err`.
    pub fn write<T>(
        &mut self,
        f: impl FnOnce(&mut WriteTx<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut write = WriteTx { tx };
        let value = f(&mut write)?;
        write.tx.commit()?;
        Ok(value)
    }
}

/// An open write transaction. Dropped without commit = rolled back.
pub struct WriteTx<'a> {
    tx: rusqlite::Transaction<'a>,
}

impl std::fmt::Debug for WriteTx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteTx")
    }
}

impl WriteTx<'_> {
    fn reader(&self) -> Reader<'_> {
        Reader(&self.tx)
    }

    /// Upserts a `meta` value.
    pub fn meta_set(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        self.tx
            .prepare_cached(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?
            .execute(params![key, value])?;
        Ok(())
    }

    /// Upserts the `local_auth` row (id = 1).
    pub fn local_auth_put(&mut self, row: &LocalAuthRow) -> Result<(), StoreError> {
        self.tx
            .prepare_cached(
                "INSERT INTO local_auth (id, personal_vault_id, kdf, kdf_salt, kdf_email, updated_at)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                   personal_vault_id = excluded.personal_vault_id,
                   kdf = excluded.kdf,
                   kdf_salt = excluded.kdf_salt,
                   kdf_email = excluded.kdf_email,
                   updated_at = excluded.updated_at",
            )?
            .execute(params![
                &row.personal_vault_id.as_bytes()[..],
                row.kdf_json,
                &row.kdf_salt[..],
                row.kdf_email,
                row.updated_at,
            ])?;
        Ok(())
    }

    /// Upserts a vault.
    pub fn vault_put(&mut self, row: &VaultRow) -> Result<(), StoreError> {
        self.tx
            .prepare_cached(
                "INSERT INTO vaults (vault_id, kind, status, role, current_epoch, bound, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (vault_id) DO UPDATE SET
                   kind = excluded.kind,
                   status = excluded.status,
                   role = excluded.role,
                   current_epoch = excluded.current_epoch,
                   bound = excluded.bound,
                   created_at = excluded.created_at",
            )?
            .execute(params![
                &row.vault_id.as_bytes()[..],
                vault_kind_str(row.kind),
                vault_status_str(row.status),
                row.role.map(role_str),
                i64::from(row.current_epoch),
                row.bound,
                row.created_at,
            ])?;
        Ok(())
    }

    /// Upserts a key wrap.
    pub fn key_wrap_put(&mut self, row: &KeyWrapRow) -> Result<(), StoreError> {
        self.tx
            .prepare_cached(
                "INSERT INTO key_wraps (vault_id, epoch, kind, sender_user_id, hpke_enc, wrap)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (vault_id, epoch, kind) DO UPDATE SET
                   sender_user_id = excluded.sender_user_id,
                   hpke_enc = excluded.hpke_enc,
                   wrap = excluded.wrap",
            )?
            .execute(params![
                &row.vault_id.as_bytes()[..],
                i64::from(row.epoch),
                row.kind.as_str(),
                row.sender_user_id.as_ref().map(|id| id.as_bytes().to_vec()),
                row.hpke_enc,
                row.wrap,
            ])?;
        Ok(())
    }

    /// Deletes a key wrap (no error when it does not exist).
    pub fn key_wrap_delete(
        &mut self,
        vault_id: &Uuid,
        epoch: u32,
        kind: WrapKind,
    ) -> Result<(), StoreError> {
        self.tx
            .prepare_cached(
                "DELETE FROM key_wraps WHERE vault_id = ?1 AND epoch = ?2 AND kind = ?3",
            )?
            .execute(params![
                &vault_id.as_bytes()[..],
                i64::from(epoch),
                kind.as_str()
            ])?;
        Ok(())
    }

    /// Upserts a record row. Decodes the envelope header (checks 1-3 of
    /// docs/storage-format.md, section 5.4) and refuses with
    /// [`StoreError::Corrupt`] when class, vault_id, record_id, revision,
    /// key_epoch or tombstone differ from the columns (section 4.6). A
    /// `base_envelope` must be a valid envelope of the same record.
    pub fn record_put(&mut self, row: &RecordRow) -> Result<(), StoreError> {
        let header = wire::split(&row.envelope)
            .map_err(|_| StoreError::Corrupt("envelope header is invalid"))?
            .header;
        if header.class != row.class
            || header.vault_id != *row.vault_id.as_bytes()
            || header.record_id != *row.record_id.as_bytes()
            || header.revision != row.revision
            || header.key_epoch != row.key_epoch
            || header.tombstone != row.tombstone
        {
            return Err(StoreError::Corrupt("envelope header and columns differ"));
        }
        if let Some(base) = &row.base_envelope {
            let base = wire::split(base)
                .map_err(|_| StoreError::Corrupt("base envelope header is invalid"))?
                .header;
            if base.vault_id != header.vault_id || base.record_id != header.record_id {
                return Err(StoreError::Corrupt(
                    "base envelope belongs to another record",
                ));
            }
        }
        let revision = u64_to_db(row.revision, "revision out of range")?;
        let synced_revision = u64_to_db(row.synced_revision, "synced revision out of range")?;
        self.tx
            .prepare_cached(
                "INSERT INTO records (record_id, vault_id, class, revision, synced_revision,
                   key_epoch, tombstone, tombstoned_at, server_seq, envelope, base_envelope)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (vault_id, record_id) DO UPDATE SET
                   class = excluded.class,
                   revision = excluded.revision,
                   synced_revision = excluded.synced_revision,
                   key_epoch = excluded.key_epoch,
                   tombstone = excluded.tombstone,
                   tombstoned_at = excluded.tombstoned_at,
                   server_seq = excluded.server_seq,
                   envelope = excluded.envelope,
                   base_envelope = excluded.base_envelope",
            )?
            .execute(params![
                &row.record_id.as_bytes()[..],
                &row.vault_id.as_bytes()[..],
                i64::from(row.class),
                revision,
                synced_revision,
                i64::from(row.key_epoch),
                row.tombstone,
                row.tombstoned_at,
                row.server_seq,
                row.envelope,
                row.base_envelope,
            ])?;
        Ok(())
    }

    /// Deletes a record row (no error when it does not exist).
    pub fn record_delete(&mut self, vault_id: &Uuid, record_id: &Uuid) -> Result<(), StoreError> {
        self.tx
            .prepare_cached("DELETE FROM records WHERE vault_id = ?1 AND record_id = ?2")?
            .execute(params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..]])?;
        Ok(())
    }

    /// Upserts a `local_settings` value. `value_json` must be valid JSON.
    pub fn setting_put(&mut self, key: &str, value_json: &str) -> Result<(), StoreError> {
        serde_json::from_str::<serde::de::IgnoredAny>(value_json)
            .map_err(|_| StoreError::Corrupt("setting value is not JSON"))?;
        self.tx
            .prepare_cached(
                "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?
            .execute(params![key, value_json])?;
        Ok(())
    }

    /// Deletes a `local_settings` row (no error when it does not exist).
    pub fn setting_delete(&mut self, key: &str) -> Result<(), StoreError> {
        self.tx
            .prepare_cached("DELETE FROM local_settings WHERE key = ?1")?
            .execute(params![key])?;
        Ok(())
    }

    /// A `meta` value, read inside the transaction.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.reader().meta_get(key)
    }

    /// The `local_auth` row, read inside the transaction.
    pub fn local_auth(&self) -> Result<Option<LocalAuthRow>, StoreError> {
        self.reader().local_auth()
    }

    /// One vault, read inside the transaction.
    pub fn vault(&self, vault_id: &Uuid) -> Result<Option<VaultRow>, StoreError> {
        self.reader().vault(vault_id)
    }

    /// Every vault, read inside the transaction.
    pub fn vaults(&self) -> Result<Vec<VaultRow>, StoreError> {
        self.reader().vaults()
    }

    /// One key wrap, read inside the transaction.
    pub fn key_wrap(
        &self,
        vault_id: &Uuid,
        epoch: u32,
        kind: WrapKind,
    ) -> Result<Option<KeyWrapRow>, StoreError> {
        self.reader().key_wrap(vault_id, epoch, kind)
    }

    /// Every key wrap of a vault, read inside the transaction.
    pub fn key_wraps(&self, vault_id: &Uuid) -> Result<Vec<KeyWrapRow>, StoreError> {
        self.reader().key_wraps(vault_id)
    }

    /// One record row, read inside the transaction.
    pub fn record(
        &self,
        vault_id: &Uuid,
        record_id: &Uuid,
    ) -> Result<Option<RecordRow>, StoreError> {
        self.reader().record(vault_id, record_id)
    }

    /// Rows of a vault, read inside the transaction.
    pub fn records(
        &self,
        vault_id: &Uuid,
        classes: Option<&[u8]>,
    ) -> Result<Vec<RecordRow>, StoreError> {
        self.reader().records(vault_id, classes)
    }

    /// A `local_settings` value, read inside the transaction.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.reader().setting(key)
    }

    /// Every `local_settings` row, read inside the transaction.
    pub fn settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        self.reader().settings()
    }
}

/// A `records` row of [`Store::records_for_load`] that could not be converted.
#[derive(Debug)]
pub struct UnreadableRow {
    /// The record id, when the `record_id` column is a 16-byte BLOB.
    pub record_id: Option<Uuid>,
    pub error: StoreError,
}

/// Read queries shared by [`Store`] and [`WriteTx`].
struct Reader<'c>(&'c Connection);

type RawVault = (Vec<u8>, String, String, Option<String>, i64, bool, String);
type RawKeyWrap = (
    Vec<u8>,
    i64,
    String,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Vec<u8>,
);

struct RawRecord {
    record_id: Vec<u8>,
    vault_id: Vec<u8>,
    class: i64,
    revision: i64,
    synced_revision: i64,
    key_epoch: i64,
    tombstone: bool,
    tombstoned_at: Option<String>,
    server_seq: Option<i64>,
    envelope: Vec<u8>,
    base_envelope: Option<Vec<u8>>,
}

const RECORD_COLUMNS: &str = "record_id, vault_id, class, revision, synced_revision, key_epoch,
    tombstone, tombstoned_at, server_seq, envelope, base_envelope";

fn raw_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRecord> {
    Ok(RawRecord {
        record_id: row.get(0)?,
        vault_id: row.get(1)?,
        class: row.get(2)?,
        revision: row.get(3)?,
        synced_revision: row.get(4)?,
        key_epoch: row.get(5)?,
        tombstone: row.get(6)?,
        tombstoned_at: row.get(7)?,
        server_seq: row.get(8)?,
        envelope: row.get(9)?,
        base_envelope: row.get(10)?,
    })
}

/// Column values of one `records` row, read without type conversion.
type LooseRecord = [SqlValue; 11];

fn loose_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<LooseRecord> {
    let values = (0..11)
        .map(|index| row.get::<_, SqlValue>(index))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    values
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidColumnIndex(11))
}

const WRONG_TYPE: StoreError = StoreError::Corrupt("column has the wrong type");

fn loose_blob(value: SqlValue) -> Result<Vec<u8>, StoreError> {
    match value {
        SqlValue::Blob(bytes) => Ok(bytes),
        _ => Err(WRONG_TYPE),
    }
}

fn loose_opt_blob(value: SqlValue) -> Result<Option<Vec<u8>>, StoreError> {
    match value {
        SqlValue::Null => Ok(None),
        other => loose_blob(other).map(Some),
    }
}

fn loose_int(value: &SqlValue) -> Result<i64, StoreError> {
    match value {
        SqlValue::Integer(number) => Ok(*number),
        _ => Err(WRONG_TYPE),
    }
}

fn loose_opt_int(value: &SqlValue) -> Result<Option<i64>, StoreError> {
    match value {
        SqlValue::Null => Ok(None),
        other => loose_int(other).map(Some),
    }
}

fn loose_opt_text(value: SqlValue) -> Result<Option<String>, StoreError> {
    match value {
        SqlValue::Null => Ok(None),
        SqlValue::Text(text) => Ok(Some(text)),
        _ => Err(WRONG_TYPE),
    }
}

/// Converts one loosely read row; the error carries the record id when readable.
fn loose_record_row(values: LooseRecord) -> Result<RecordRow, UnreadableRow> {
    let record_id = match &values[0] {
        SqlValue::Blob(bytes) => Uuid::from_slice(bytes).ok(),
        _ => None,
    };
    let convert = |values: LooseRecord| -> Result<RecordRow, StoreError> {
        let [
            id,
            vault,
            class,
            revision,
            synced_revision,
            key_epoch,
            tombstone,
            tombstoned_at,
            server_seq,
            envelope,
            base_envelope,
        ] = values;
        record_row(RawRecord {
            record_id: loose_blob(id)?,
            vault_id: loose_blob(vault)?,
            class: loose_int(&class)?,
            revision: loose_int(&revision)?,
            synced_revision: loose_int(&synced_revision)?,
            key_epoch: loose_int(&key_epoch)?,
            tombstone: loose_int(&tombstone)? != 0,
            tombstoned_at: loose_opt_text(tombstoned_at)?,
            server_seq: loose_opt_int(&server_seq)?,
            envelope: loose_blob(envelope)?,
            base_envelope: loose_opt_blob(base_envelope)?,
        })
    };
    convert(values).map_err(|error| UnreadableRow { record_id, error })
}

fn record_row(raw: RawRecord) -> Result<RecordRow, StoreError> {
    Ok(RecordRow {
        vault_id: uuid_from_blob(&raw.vault_id)?,
        record_id: uuid_from_blob(&raw.record_id)?,
        class: u8::try_from(raw.class).map_err(|_| StoreError::Corrupt("class out of range"))?,
        revision: u64_from_db(raw.revision, "negative revision")?,
        synced_revision: u64_from_db(raw.synced_revision, "negative synced revision")?,
        key_epoch: u32_from_db(raw.key_epoch, "key epoch out of range")?,
        tombstone: raw.tombstone,
        tombstoned_at: raw.tombstoned_at,
        server_seq: raw.server_seq,
        envelope: raw.envelope,
        base_envelope: raw.base_envelope,
    })
}

fn vault_row(raw: RawVault) -> Result<VaultRow, StoreError> {
    let (vault_id, kind, status, role, current_epoch, bound, created_at) = raw;
    Ok(VaultRow {
        vault_id: uuid_from_blob(&vault_id)?,
        kind: parse_vault_kind(&kind)?,
        status: parse_vault_status(&status)?,
        role: role.as_deref().map(parse_role).transpose()?,
        current_epoch: u32_from_db(current_epoch, "epoch out of range")?,
        bound,
        created_at,
    })
}

fn key_wrap_row(raw: RawKeyWrap) -> Result<KeyWrapRow, StoreError> {
    let (vault_id, epoch, kind, sender_user_id, hpke_enc, wrap) = raw;
    Ok(KeyWrapRow {
        vault_id: uuid_from_blob(&vault_id)?,
        epoch: u32_from_db(epoch, "epoch out of range")?,
        kind: WrapKind::parse(&kind).ok_or(StoreError::Corrupt("unknown wrap kind"))?,
        sender_user_id: sender_user_id.as_deref().map(uuid_from_blob).transpose()?,
        hpke_enc,
        wrap,
    })
}

impl Reader<'_> {
    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .0
            .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
            .query_row(params![key], |r| r.get(0))
            .optional()?)
    }

    fn local_auth(&self) -> Result<Option<LocalAuthRow>, StoreError> {
        let raw = self
            .0
            .prepare_cached(
                "SELECT personal_vault_id, kdf, kdf_salt, kdf_email, updated_at
                 FROM local_auth WHERE id = 1",
            )?
            .query_row([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .optional()?;
        let Some((vault_id, kdf_json, salt, kdf_email, updated_at)) = raw else {
            return Ok(None);
        };
        let kdf_salt: [u8; 16] = salt
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Corrupt("kdf_salt is not 16 bytes"))?;
        Ok(Some(LocalAuthRow {
            personal_vault_id: uuid_from_blob(&vault_id)?,
            kdf_json,
            kdf_salt,
            kdf_email,
            updated_at,
        }))
    }

    fn vault_query(&self, sql: &str, id: Option<&Uuid>) -> Result<Vec<VaultRow>, StoreError> {
        let mut stmt = self.0.prepare_cached(sql)?;
        let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RawVault> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        };
        let raws: Vec<RawVault> = match id {
            Some(id) => stmt
                .query_map(params![&id.as_bytes()[..]], map)?
                .collect::<Result<_, _>>()?,
            None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
        };
        raws.into_iter().map(vault_row).collect()
    }

    fn vault(&self, vault_id: &Uuid) -> Result<Option<VaultRow>, StoreError> {
        Ok(self
            .vault_query(
                "SELECT vault_id, kind, status, role, current_epoch, bound, created_at
                 FROM vaults WHERE vault_id = ?1",
                Some(vault_id),
            )?
            .into_iter()
            .next())
    }

    fn vaults(&self) -> Result<Vec<VaultRow>, StoreError> {
        self.vault_query(
            "SELECT vault_id, kind, status, role, current_epoch, bound, created_at
             FROM vaults ORDER BY vault_id",
            None,
        )
    }

    fn key_wrap(
        &self,
        vault_id: &Uuid,
        epoch: u32,
        kind: WrapKind,
    ) -> Result<Option<KeyWrapRow>, StoreError> {
        let raw = self
            .0
            .prepare_cached(
                "SELECT vault_id, epoch, kind, sender_user_id, hpke_enc, wrap
                 FROM key_wraps WHERE vault_id = ?1 AND epoch = ?2 AND kind = ?3",
            )?
            .query_row(
                params![&vault_id.as_bytes()[..], i64::from(epoch), kind.as_str()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(key_wrap_row).transpose()
    }

    fn key_wraps(&self, vault_id: &Uuid) -> Result<Vec<KeyWrapRow>, StoreError> {
        let mut stmt = self.0.prepare_cached(
            "SELECT vault_id, epoch, kind, sender_user_id, hpke_enc, wrap
             FROM key_wraps WHERE vault_id = ?1 ORDER BY epoch, kind",
        )?;
        let raws: Vec<RawKeyWrap> = stmt
            .query_map(params![&vault_id.as_bytes()[..]], |r| {
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
        raws.into_iter().map(key_wrap_row).collect()
    }

    fn record(&self, vault_id: &Uuid, record_id: &Uuid) -> Result<Option<RecordRow>, StoreError> {
        let raw = self
            .0
            .prepare_cached(&format!(
                "SELECT {RECORD_COLUMNS} FROM records WHERE vault_id = ?1 AND record_id = ?2"
            ))?
            .query_row(
                params![&vault_id.as_bytes()[..], &record_id.as_bytes()[..]],
                raw_record,
            )
            .optional()?;
        raw.map(record_row).transpose()
    }

    fn records(
        &self,
        vault_id: &Uuid,
        classes: Option<&[u8]>,
    ) -> Result<Vec<RecordRow>, StoreError> {
        let mut raws: Vec<RawRecord> = Vec::new();
        match classes {
            None => {
                let mut stmt = self.0.prepare_cached(&format!(
                    "SELECT {RECORD_COLUMNS} FROM records WHERE vault_id = ?1 ORDER BY record_id"
                ))?;
                for raw in stmt.query_map(params![&vault_id.as_bytes()[..]], raw_record)? {
                    raws.push(raw?);
                }
            }
            Some(classes) => {
                let mut stmt = self.0.prepare_cached(&format!(
                    "SELECT {RECORD_COLUMNS} FROM records WHERE vault_id = ?1 AND class = ?2"
                ))?;
                let mut seen = Vec::with_capacity(classes.len());
                for &class in classes {
                    if seen.contains(&class) {
                        continue;
                    }
                    seen.push(class);
                    for raw in stmt.query_map(
                        params![&vault_id.as_bytes()[..], i64::from(class)],
                        raw_record,
                    )? {
                        raws.push(raw?);
                    }
                }
                raws.sort_by(|a, b| a.record_id.cmp(&b.record_id));
            }
        }
        raws.into_iter().map(record_row).collect()
    }

    fn records_for_load(
        &self,
        vault_id: &Uuid,
    ) -> Result<Vec<Result<RecordRow, UnreadableRow>>, StoreError> {
        let mut stmt = self.0.prepare_cached(&format!(
            "SELECT {RECORD_COLUMNS} FROM records WHERE vault_id = ?1 ORDER BY record_id"
        ))?;
        let mut rows = Vec::new();
        for values in stmt.query_map(params![&vault_id.as_bytes()[..]], loose_record)? {
            rows.push(loose_record_row(values?));
        }
        Ok(rows)
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .0
            .prepare_cached("SELECT value FROM local_settings WHERE key = ?1")?
            .query_row(params![key], |r| r.get(0))
            .optional()?)
    }

    fn settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        let mut stmt = self
            .0
            .prepare_cached("SELECT key, value FROM local_settings ORDER BY key")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests;
