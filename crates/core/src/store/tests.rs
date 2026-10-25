//! Unit tests of the store (open, pragmas, migrations, backups, rows).

use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use shellpp_sync_api::classes;
use shellpp_sync_api::envelope::EnvelopeHeader;
use tempfile::TempDir;
use uuid::Uuid;

use super::migrate::{Migration, MigrationPlan};
use super::*;
use crate::crypto::{Key32, seal_envelope, sha256};
use crate::service::dto::{Role, VaultKind, VaultStatus};

const T0: u64 = 1_792_362_665; // 2026-10-18T22:31:05Z

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

// A future schema: the real steps of this app, then a test step from its
// version (2) to 3.
static V2_STEPS: [Migration; 2] = [
    super::migrate::MIGRATIONS[0],
    Migration {
        from: 2,
        sql: "CREATE TABLE test_extra (id INTEGER PRIMARY KEY, note TEXT NOT NULL DEFAULT '');",
    },
];

const V2_PLAN: MigrationPlan = MigrationPlan {
    v1_sql: SCHEMA_V1_SQL,
    steps: &V2_STEPS,
};

static FAILING_STEPS: [Migration; 2] = [
    super::migrate::MIGRATIONS[0],
    Migration {
        from: 2,
        // `meta` already exists: the step fails after creating `half_done`.
        sql: "CREATE TABLE half_done (id INTEGER); CREATE TABLE meta (x INTEGER);",
    },
];

const FAILING_PLAN: MigrationPlan = MigrationPlan {
    v1_sql: SCHEMA_V1_SQL,
    steps: &FAILING_STEPS,
};

fn db_file(dir: &Path) -> std::path::PathBuf {
    dir.join(crate::ids::DB_FILE_NAME)
}

fn table_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn backups(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("backups")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn personal_vault(id: Uuid) -> VaultRow {
    VaultRow {
        vault_id: id,
        kind: VaultKind::Personal,
        status: VaultStatus::Active,
        role: None,
        current_epoch: 1,
        bound: false,
        created_at: "2026-10-18T22:31:05.123Z".into(),
    }
}

fn header(vault_id: Uuid, record_id: Uuid) -> EnvelopeHeader {
    EnvelopeHeader {
        class: classes::HOSTS,
        tombstone: false,
        vault_id: *vault_id.as_bytes(),
        record_id: *record_id.as_bytes(),
        revision: 1,
        key_epoch: 1,
    }
}

fn record(key: &Key32, h: &EnvelopeHeader) -> RecordRow {
    RecordRow {
        vault_id: Uuid::from_bytes(h.vault_id),
        record_id: Uuid::from_bytes(h.record_id),
        class: h.class,
        revision: h.revision,
        synced_revision: 0,
        key_epoch: h.key_epoch,
        tombstone: h.tombstone,
        tombstoned_at: None,
        server_seq: None,
        envelope: seal_envelope(key, h, br#"{"v":1}"#).unwrap(),
        base_envelope: None,
    }
}

#[test]
fn create_then_reopen() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("data");
    let (store, outcome) = Store::open(&dir, "0.1.0").unwrap();
    assert_eq!(outcome, OpenOutcome::Created);
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.data_dir(), Some(dir.as_path()));
    assert_eq!(store.db_path(), Some(db_file(&dir)));
    assert_eq!(
        store.meta_get(META_LAST_APP_VERSION).unwrap().as_deref(),
        Some("0.1.0")
    );
    assert_eq!(
        table_names(&store.conn),
        [
            "duplicate_pairs",
            "key_wraps",
            "local_auth",
            "local_settings",
            "meta",
            "recently_deleted",
            "records",
            "resync_returned",
            "server_account",
            "server_tls",
            "sync_state",
            "vaults"
        ]
    );
    assert_eq!(format!("{store:?}"), "Store { in_memory: false }");
    drop(store);
    assert!(backups(&dir).is_empty(), "no backup for a new database");

    let (store, outcome) = Store::open(&dir, "0.1.1").unwrap();
    assert_eq!(outcome, OpenOutcome::Opened);
    assert_eq!(
        store.meta_get(META_LAST_APP_VERSION).unwrap().as_deref(),
        Some("0.1.1")
    );
    assert!(backups(&dir).is_empty());
}

#[test]
fn pragmas_are_applied() {
    let tmp = TempDir::new().unwrap();
    let (store, _) = Store::open(tmp.path(), "0.1.0").unwrap();
    let c = &store.conn;
    let journal: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    let fk: i64 = c
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    let secure: i64 = c
        .query_row("PRAGMA secure_delete", [], |r| r.get(0))
        .unwrap();
    assert_eq!(secure, 1);
    let sync: i64 = c.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
    assert_eq!(sync, 2, "FULL");
    let busy: i64 = c
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(busy, 5000);
    let limit: i64 = c
        .query_row("PRAGMA journal_size_limit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(limit, 0);
}

#[test]
fn an_open_store_locks_its_data_directory() {
    let tmp = TempDir::new().unwrap();
    let lock_path = tmp.path().join(dir_lock::LOCK_FILE_NAME);
    let (store, _) = Store::open(tmp.path(), "0.1.0").unwrap();
    // Another process (another handle) cannot take the lock.
    let other = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(matches!(
        other.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(store);
    other.try_lock().expect("released when the store closes");
    // While another process holds it, opening fails after the wait.
    let error = Store::open(tmp.path(), "0.1.0").unwrap_err();
    assert!(matches!(error, StoreError::InUse));
    assert_eq!(error.code(), "storage_error");
    drop(other);
    Store::open(tmp.path(), "0.1.0").expect("opens again");
}

#[test]
fn unreadable_rows_are_reported_one_by_one() {
    let tmp = TempDir::new().unwrap();
    let (mut store, _) = Store::open(tmp.path(), "0.1.0").unwrap();
    let vault = Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0001);
    let ids = [
        Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0010),
        Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0011),
        Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0012),
    ];
    store
        .write(|tx| {
            tx.vault_put(&VaultRow {
                vault_id: vault,
                kind: VaultKind::Personal,
                status: VaultStatus::Active,
                role: None,
                current_epoch: 1,
                bound: false,
                created_at: "2026-10-18T22:31:05Z".into(),
            })
        })
        .unwrap();
    for id in ids {
        store
            .conn
            .execute(
                "INSERT INTO records (record_id, vault_id, class, revision, key_epoch, envelope)
                 VALUES (?1, ?2, 2, 1, 1, x'00')",
                rusqlite::params![&id.as_bytes()[..], &vault.as_bytes()[..]],
            )
            .unwrap();
    }
    // A negative revision and a text epoch: the strict reader fails as a whole.
    store
        .conn
        .execute(
            "UPDATE records SET revision = -1 WHERE record_id = ?1",
            [&ids[1].as_bytes()[..]],
        )
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE records SET key_epoch = 'one' WHERE record_id = ?1",
            [&ids[2].as_bytes()[..]],
        )
        .unwrap();
    assert!(store.records(&vault, None).is_err());

    let rows = store.records_for_load(&vault).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].as_ref().unwrap().record_id, ids[0]);
    for (row, id) in rows[1..].iter().zip(&ids[1..]) {
        let unreadable = row.as_ref().unwrap_err();
        assert_eq!(unreadable.record_id, Some(*id));
        assert_eq!(unreadable.error.code(), "damaged");
    }
}

#[test]
fn checkpoint_truncates_the_wal() {
    let tmp = TempDir::new().unwrap();
    let (mut store, _) = Store::open(tmp.path(), "0.1.0").unwrap();
    let wal = tmp.path().join("shellpp.db-wal");
    store
        .write(|tx| tx.setting_put("view.hosts.layout", "\"list\""))
        .unwrap();
    assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    assert!(store.checkpoint_truncate().unwrap());
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
    // The data is in the database file now.
    assert_eq!(
        store.setting("view.hosts.layout").unwrap().as_deref(),
        Some("\"list\"")
    );
    drop(store);
    let (store, _) = Store::open(tmp.path(), "0.1.0").unwrap();
    assert_eq!(
        store.setting("view.hosts.layout").unwrap().as_deref(),
        Some("\"list\"")
    );
    // A no-op in memory.
    assert!(
        Store::open_in_memory()
            .unwrap()
            .checkpoint_truncate()
            .unwrap()
    );
}

#[test]
fn in_memory_store() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.data_dir(), None);
    assert_eq!(store.db_path(), None);
    let fk: i64 = store
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    assert_eq!(format!("{store:?}"), "Store { in_memory: true }");
}

#[cfg(unix)]
#[test]
fn unix_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("data");
    let (store, _) = Store::open(&dir, "0.1.0").unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&db_file(&dir)), 0o600);
    let wal = dir.join("shellpp.db-wal");
    if wal.exists() {
        assert_eq!(mode(&wal), 0o600);
    }
    drop(store);
    // A migration backup is private too.
    let (_, outcome) = Store::open_with(&dir, "0.2.0", &V2_PLAN, at(T0)).unwrap();
    let OpenOutcome::Migrated { backup, .. } = outcome else {
        panic!("expected a migration");
    };
    assert_eq!(mode(&backup), 0o600);
    assert_eq!(mode(&dir.join("backups")), 0o700);
}

#[test]
fn newer_schema_is_refused_and_the_file_is_unchanged() {
    let tmp = TempDir::new().unwrap();
    let path = db_file(tmp.path());
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); PRAGMA user_version = 3;")
            .unwrap();
    }
    let before = sha256(&std::fs::read(&path).unwrap());
    let err = Store::open(tmp.path(), "0.1.0").unwrap_err();
    assert!(matches!(err, StoreError::NewerSchema { found: 3 }));
    assert_eq!(err.code(), "newer_schema");
    assert_eq!(sha256(&std::fs::read(&path).unwrap()), before);
    assert!(!tmp.path().join("shellpp.db-wal").exists());
    assert!(backups(tmp.path()).is_empty());
}

#[test]
fn newer_schema_in_wal_mode_is_refused() {
    let tmp = TempDir::new().unwrap();
    let path = db_file(tmp.path());
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); PRAGMA user_version = 7;")
            .unwrap();
    }
    let before = sha256(&std::fs::read(&path).unwrap());
    assert!(matches!(
        Store::open(tmp.path(), "0.1.0"),
        Err(StoreError::NewerSchema { found: 7 })
    ));
    assert_eq!(sha256(&std::fs::read(&path).unwrap()), before);
}

#[test]
fn unversioned_database_with_tables_is_refused() {
    let tmp = TempDir::new().unwrap();
    {
        let conn = Connection::open(db_file(tmp.path())).unwrap();
        conn.execute_batch("CREATE TABLE other (x INTEGER);")
            .unwrap();
    }
    let path = db_file(tmp.path());
    let before = sha256(&std::fs::read(&path).unwrap());
    assert!(matches!(
        Store::open(tmp.path(), "0.1.0"),
        Err(StoreError::Corrupt(_))
    ));
    assert_eq!(sha256(&std::fs::read(&path).unwrap()), before);
}

#[test]
fn migration_writes_a_backup_first() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let vault_id = Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0001);
    {
        let (mut store, _) = Store::open(dir, "0.1.0").unwrap();
        store
            .write(|tx| tx.vault_put(&personal_vault(vault_id)))
            .unwrap();
    }

    let (store, outcome) = Store::open_with(dir, "0.2.0", &V2_PLAN, at(T0)).unwrap();
    let expected_backup = dir.join("backups").join("shellpp-20261018T223105Z.db");
    assert_eq!(
        outcome,
        OpenOutcome::Migrated {
            from: 2,
            backup: expected_backup.clone()
        }
    );
    assert_eq!(store.schema_version().unwrap(), 3);
    assert!(table_names(&store.conn).contains(&"test_extra".to_owned()));
    assert_eq!(store.vault(&vault_id).unwrap().unwrap().vault_id, vault_id);
    assert_eq!(
        store.meta_get(META_LAST_APP_VERSION).unwrap().as_deref(),
        Some("0.2.0")
    );

    // The backup is a consistent copy of version 2 with the data.
    let copy = Connection::open(&expected_backup).unwrap();
    assert_eq!(migrate::user_version(&copy).unwrap(), 2);
    assert!(!table_names(&copy).contains(&"test_extra".to_owned()));
    let n: i64 = copy
        .query_row("SELECT count(*) FROM vaults", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    drop(copy);
    drop(store);

    // The current app (schema 2) refuses the migrated database.
    assert!(matches!(
        Store::open(dir, "0.1.0"),
        Err(StoreError::NewerSchema { found: 3 })
    ));
    // Reopening with the plan that produced it does nothing new.
    let (_, outcome) = Store::open_with(dir, "0.2.0", &V2_PLAN, at(T0 + 5)).unwrap();
    assert_eq!(outcome, OpenOutcome::Opened);
    assert_eq!(backups(dir).len(), 1);
}

#[test]
fn new_database_is_created_at_the_newest_version_without_backup() {
    let tmp = TempDir::new().unwrap();
    let (store, outcome) = Store::open_with(tmp.path(), "0.2.0", &V2_PLAN, at(T0)).unwrap();
    assert_eq!(outcome, OpenOutcome::Created);
    assert_eq!(store.schema_version().unwrap(), 3);
    assert!(backups(tmp.path()).is_empty());
}

/// Puts the database back to version 2 so the test plan migrates it again.
fn downgrade_to_v1(dir: &Path) {
    let conn = Connection::open(db_file(dir)).unwrap();
    conn.execute_batch("DROP TABLE test_extra; PRAGMA user_version = 2;")
        .unwrap();
}

#[test]
fn only_the_three_newest_backups_are_kept() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    drop(Store::open(dir, "0.1.0").unwrap());
    std::fs::create_dir_all(dir.join("backups")).unwrap();
    std::fs::write(dir.join("backups").join("keep-me.txt"), b"not a backup").unwrap();

    for i in 0..5u64 {
        if i > 0 {
            downgrade_to_v1(dir);
        }
        let (_, outcome) = Store::open_with(dir, "0.2.0", &V2_PLAN, at(T0 + i * 3600)).unwrap();
        assert!(matches!(outcome, OpenOutcome::Migrated { from: 2, .. }));
    }
    assert_eq!(
        backups(dir),
        [
            "keep-me.txt",
            "shellpp-20261019T003105Z.db",
            "shellpp-20261019T013105Z.db",
            "shellpp-20261019T023105Z.db",
        ]
    );
}

#[test]
fn backup_names_never_collide() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    drop(Store::open(dir, "0.1.0").unwrap());
    drop(Store::open_with(dir, "0.2.0", &V2_PLAN, at(T0)).unwrap());
    downgrade_to_v1(dir);
    let (_, outcome) = Store::open_with(dir, "0.2.0", &V2_PLAN, at(T0)).unwrap();
    assert_eq!(
        outcome,
        OpenOutcome::Migrated {
            from: 2,
            backup: dir.join("backups").join("shellpp-20261018T223106Z.db"),
        }
    );
    assert_eq!(
        backups(dir),
        ["shellpp-20261018T223105Z.db", "shellpp-20261018T223106Z.db"]
    );
}

#[test]
fn failed_migration_step_is_rolled_back() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    drop(Store::open(dir, "0.1.0").unwrap());
    let err = Store::open_with(dir, "0.2.0", &FAILING_PLAN, at(T0)).unwrap_err();
    assert!(matches!(err, StoreError::Migration { to: 3 }));
    assert_eq!(err.code(), "storage_error");
    assert_eq!(backups(dir), ["shellpp-20261018T223105Z.db"]);
    let conn = Connection::open(db_file(dir)).unwrap();
    assert_eq!(migrate::user_version(&conn).unwrap(), 2);
    assert!(!table_names(&conn).contains(&"half_done".to_owned()));
    drop(conn);
    // The app of version 2 still opens it.
    let (_, outcome) = Store::open(dir, "0.1.0").unwrap();
    assert_eq!(outcome, OpenOutcome::Opened);
}

#[test]
fn write_rolls_back_on_error() {
    let mut store = Store::open_in_memory().unwrap();
    let err = store
        .write(|tx| {
            tx.meta_set(META_DB_ID, "0199f6a1-b2c3-7d4e-8f00-112233445566")?;
            tx.setting_put("security.auto_lock_minutes", "15")?;
            assert!(
                tx.meta_get(META_DB_ID)?.is_some(),
                "visible inside the transaction"
            );
            Err::<(), _>(StoreError::Corrupt("test"))
        })
        .unwrap_err();
    assert!(matches!(err, StoreError::Corrupt("test")));
    assert_eq!(store.meta_get(META_DB_ID).unwrap(), None);
    assert_eq!(store.setting("security.auto_lock_minutes").unwrap(), None);

    store
        .write(|tx| tx.meta_set(META_DB_ID, "0199f6a1-b2c3-7d4e-8f00-112233445566"))
        .unwrap();
    assert!(store.meta_get(META_DB_ID).unwrap().is_some());
}

#[test]
fn local_auth_vaults_and_key_wraps() {
    let mut store = Store::open_in_memory().unwrap();
    let vault_id = Uuid::from_u128(0xAA);
    let team_id = Uuid::from_u128(0xBB);
    assert_eq!(store.local_auth().unwrap(), None);

    let auth = LocalAuthRow {
        personal_vault_id: vault_id,
        kdf_json: crate::crypto::KdfParams::preset(crate::crypto::PlatformClass::Desktop).to_json(),
        kdf_salt: [7; 16],
        kdf_email: None,
        updated_at: "2026-10-18T22:31:05.123Z".into(),
    };
    let team = VaultRow {
        vault_id: team_id,
        kind: VaultKind::Team,
        status: VaultStatus::Archived,
        role: Some(Role::Viewer),
        current_epoch: 4,
        bound: true,
        created_at: "2026-10-18T22:31:06.000Z".into(),
    };
    let kek = KeyWrapRow {
        vault_id,
        epoch: 1,
        kind: WrapKind::Kek,
        sender_user_id: None,
        hpke_enc: None,
        wrap: vec![1; 73],
    };
    let hpke = KeyWrapRow {
        vault_id: team_id,
        epoch: 4,
        kind: WrapKind::Hpke,
        sender_user_id: Some(Uuid::from_u128(0xCC)),
        hpke_enc: Some(vec![2; 32]),
        wrap: vec![3; 48],
    };
    store
        .write(|tx| {
            tx.vault_put(&personal_vault(vault_id))?;
            tx.vault_put(&team)?;
            tx.local_auth_put(&auth)?;
            tx.key_wrap_put(&kek)?;
            tx.key_wrap_put(&hpke)
        })
        .unwrap();

    assert_eq!(store.local_auth().unwrap().as_ref(), Some(&auth));
    assert_eq!(store.vault(&team_id).unwrap().as_ref(), Some(&team));
    assert_eq!(
        store.vaults().unwrap(),
        [personal_vault(vault_id), team.clone()]
    );
    assert_eq!(
        store
            .key_wrap(&vault_id, 1, WrapKind::Kek)
            .unwrap()
            .as_ref(),
        Some(&kek)
    );
    assert_eq!(
        store.key_wrap(&vault_id, 1, WrapKind::Recovery).unwrap(),
        None
    );
    assert_eq!(
        store.key_wraps(&team_id).unwrap(),
        std::slice::from_ref(&hpke)
    );

    // Upserts replace.
    let auth2 = LocalAuthRow {
        kdf_salt: [8; 16],
        kdf_email: Some("alice@example.com".into()),
        ..auth
    };
    let kek2 = KeyWrapRow {
        wrap: vec![9; 73],
        ..kek.clone()
    };
    let mut vault2 = personal_vault(vault_id);
    vault2.current_epoch = 2;
    store
        .write(|tx| {
            tx.local_auth_put(&auth2)?;
            tx.key_wrap_put(&kek2)?;
            tx.vault_put(&vault2)
        })
        .unwrap();
    assert_eq!(store.local_auth().unwrap(), Some(auth2));
    assert_eq!(store.key_wraps(&vault_id).unwrap(), [kek2]);
    assert_eq!(store.vault(&vault_id).unwrap().unwrap().current_epoch, 2);

    store
        .write(|tx| tx.key_wrap_delete(&vault_id, 1, WrapKind::Kek))
        .unwrap();
    assert!(store.key_wraps(&vault_id).unwrap().is_empty());

    // Foreign keys: a wrap of an unknown vault is refused.
    let orphan = KeyWrapRow {
        vault_id: Uuid::from_u128(0xDD),
        ..kek
    };
    assert!(matches!(
        store.write(|tx| tx.key_wrap_put(&orphan)),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn settings_rows() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .write(|tx| {
            tx.setting_put("view.hosts.layout", r#""list""#)?;
            tx.setting_put("security.auto_lock_minutes", "15")?;
            tx.setting_put("security.auto_lock_minutes", "null")
        })
        .unwrap();
    assert_eq!(
        store.settings().unwrap(),
        [
            ("security.auto_lock_minutes".to_owned(), "null".to_owned()),
            ("view.hosts.layout".to_owned(), r#""list""#.to_owned()),
        ]
    );
    assert!(matches!(
        store.write(|tx| tx.setting_put("view.hosts.sort", "name")),
        Err(StoreError::Corrupt(_))
    ));
    store
        .write(|tx| tx.setting_delete("view.hosts.layout"))
        .unwrap();
    assert_eq!(store.setting("view.hosts.layout").unwrap(), None);
}

#[test]
fn records_round_trip_and_filters() {
    let mut store = Store::open_in_memory().unwrap();
    let key = Key32::generate().unwrap();
    let v = Uuid::from_u128(1);
    let r1 = Uuid::from_u128(0x10);
    let r2 = Uuid::from_u128(0x20);
    let r3 = Uuid::from_u128(0x30);
    let host = record(&key, &header(v, r2));
    let mut vault_meta = record(
        &key,
        &EnvelopeHeader {
            class: classes::VAULT,
            ..header(v, r1)
        },
    );
    vault_meta.synced_revision = 1;
    vault_meta.server_seq = Some(77);
    let tomb_header = EnvelopeHeader {
        tombstone: true,
        revision: 5,
        ..header(v, r3)
    };
    let mut tomb = record(&key, &tomb_header);
    tomb.tombstoned_at = Some("2026-10-18T22:31:05.123Z".into());
    tomb.base_envelope = Some(record(&key, &header(v, r3)).envelope);

    store
        .write(|tx| {
            tx.vault_put(&personal_vault(v))?;
            tx.record_put(&host)?;
            tx.record_put(&vault_meta)?;
            tx.record_put(&tomb)
        })
        .unwrap();

    assert_eq!(store.record(&v, &r2).unwrap().as_ref(), Some(&host));
    assert_eq!(store.record(&v, &r3).unwrap().as_ref(), Some(&tomb));
    assert_eq!(store.record(&Uuid::from_u128(2), &r2).unwrap(), None);
    let all = store.records(&v, None).unwrap();
    assert_eq!(all, [vault_meta.clone(), host.clone(), tomb.clone()]);
    assert_eq!(
        store.records(&v, Some(&[classes::HOSTS])).unwrap(),
        [host.clone(), tomb.clone()]
    );
    assert_eq!(
        store
            .records(&v, Some(&[classes::HOSTS, classes::VAULT, classes::HOSTS]))
            .unwrap(),
        all
    );
    assert!(
        store
            .records(&v, Some(&[classes::KEYS]))
            .unwrap()
            .is_empty()
    );
    assert!(store.records(&v, Some(&[])).unwrap().is_empty());

    // Replace with a new revision.
    let host2 = record(
        &key,
        &EnvelopeHeader {
            revision: 2,
            ..header(v, r2)
        },
    );
    store.write(|tx| tx.record_put(&host2)).unwrap();
    assert_eq!(store.record(&v, &r2).unwrap(), Some(host2));

    store.write(|tx| tx.record_delete(&v, &r2)).unwrap();
    assert_eq!(store.record(&v, &r2).unwrap(), None);
    assert_eq!(store.records(&v, None).unwrap().len(), 2);
}

#[test]
fn record_put_refuses_header_column_mismatch() {
    let mut store = Store::open_in_memory().unwrap();
    let key = Key32::generate().unwrap();
    let v = Uuid::from_u128(1);
    let r = Uuid::from_u128(2);
    store.write(|tx| tx.vault_put(&personal_vault(v))).unwrap();
    let good = record(&key, &header(v, r));

    let mut cases: Vec<RecordRow> = Vec::new();
    let mut c = good.clone();
    c.class = classes::KEYS;
    cases.push(c);
    let mut c = good.clone();
    c.vault_id = Uuid::from_u128(9);
    cases.push(c);
    let mut c = good.clone();
    c.record_id = Uuid::from_u128(9);
    cases.push(c);
    let mut c = good.clone();
    c.revision = 2;
    cases.push(c);
    let mut c = good.clone();
    c.key_epoch = 2;
    cases.push(c);
    let mut c = good.clone();
    c.tombstone = true;
    cases.push(c);
    let mut c = good.clone();
    c.envelope.truncate(343);
    cases.push(c);
    let mut c = good.clone();
    c.envelope[0] = 0x02;
    cases.push(c);
    let mut c = good.clone();
    c.base_envelope = Some(record(&key, &header(v, Uuid::from_u128(3))).envelope);
    cases.push(c);
    let mut c = good.clone();
    c.base_envelope = Some(vec![1, 2, 3]);
    cases.push(c);

    for case in &cases {
        let err = store.write(|tx| tx.record_put(case)).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt(_)), "{case:?}");
        assert_eq!(err.code(), "damaged");
    }
    assert!(store.records(&v, None).unwrap().is_empty());
    store.write(|tx| tx.record_put(&good)).unwrap();
}

#[test]
fn record_ids_are_unique_on_the_device() {
    let mut store = Store::open_in_memory().unwrap();
    let key = Key32::generate().unwrap();
    let v1 = Uuid::from_u128(1);
    let v2 = Uuid::from_u128(2);
    let r = Uuid::from_u128(0x10);
    store
        .write(|tx| {
            tx.vault_put(&personal_vault(v1))?;
            tx.vault_put(&personal_vault(v2))?;
            tx.record_put(&record(&key, &header(v1, r)))
        })
        .unwrap();
    assert!(matches!(
        store.write(|tx| tx.record_put(&record(&key, &header(v2, r)))),
        Err(StoreError::Sqlite(_))
    ));
    // A record of an unknown vault is refused by the foreign key.
    assert!(matches!(
        store.write(|tx| tx.record_put(&record(
            &key,
            &header(Uuid::from_u128(3), Uuid::from_u128(4))
        ))),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn server_account_refresh_token_may_be_null() {
    // storage-format 4.2 / 6.4: NULL while signed out after an offline recovery.
    let store = Store::open_in_memory().unwrap();
    store
        .conn
        .execute(
            "INSERT INTO server_account (id, server_url, user_id, email, refresh_token, secrets_epoch, bound_at)
             VALUES (1, 'https://sync.example.com', x'00', 'alice@example.com', NULL, 1, '2026-10-18T22:31:05.123Z')",
            [],
        )
        .unwrap();
}

#[test]
fn error_messages_have_no_paths() {
    let io = StoreError::Io(std::io::Error::other("C:\\secret\\path"));
    assert_eq!(io.to_string(), "i/o error");
    assert_eq!(io.code(), "io_error");
    assert_eq!(
        StoreError::NewerSchema { found: 3 }.to_string(),
        "database schema 3 is newer than supported"
    );
}

#[test]
fn store_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Store>();
    assert_send::<Key32>();
}

/// Run 7 plan, rule 1.7.4: a database of schema 1 with data migrates to
/// schema 2 (the sync tables, D93) with a backup first, and every row is
/// still there, unchanged.
#[test]
fn schema_1_with_data_migrates_to_2_and_keeps_everything() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let vault_id = Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0001);
    let record_id = Uuid::from_u128(0x0199_f6a1_0000_7000_8000_0000_0000_0002);
    // A schema 1 database as the app of runs 1-6 wrote it.
    {
        let mut conn = Connection::open(db_file(dir)).unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute_batch(SCHEMA_V1_SQL).unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
        conn.execute(
            "INSERT INTO vaults (vault_id, kind, status, role, current_epoch, bound, created_at)
             VALUES (?1, 'personal', 'active', NULL, 3, 0, '2026-10-01T00:00:00.000Z')",
            [&vault_id.as_bytes()[..]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO records (record_id, vault_id, class, revision, synced_revision, key_epoch,
             tombstone, envelope) VALUES (?1, ?2, 2, 7, 0, 3, 0, ?3)",
            rusqlite::params![
                &record_id.as_bytes()[..],
                &vault_id.as_bytes()[..],
                vec![1u8; 344]
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_settings (key, value) VALUES ('appearance.ui_theme', '\"dark\"')",
            [],
        )
        .unwrap();
    }
    let (store, outcome) = Store::open_with(dir, "0.7.0", &PLAN, at(T0)).unwrap();
    let OpenOutcome::Migrated { from: 1, backup } = outcome else {
        panic!("expected a migration from 1: {outcome:?}");
    };
    assert!(backup.is_file(), "the backup was written first");
    assert_eq!(store.schema_version().unwrap(), 2);
    let vault = store.vault(&vault_id).unwrap().unwrap();
    assert_eq!(vault.current_epoch, 3);
    assert_eq!(store.vault_binding(&vault_id).unwrap(), (None, None));
    let row = store.record(&vault_id, &record_id).unwrap().unwrap();
    assert_eq!((row.revision, row.key_epoch, row.class), (7, 3, 2));
    assert_eq!(row.envelope, vec![1u8; 344]);
    assert_eq!(
        store.setting("appearance.ui_theme").unwrap().as_deref(),
        Some("\"dark\"")
    );
    assert!(store.server_account().unwrap().is_none());
    assert!(store.recently_deleted().unwrap().is_empty());
    // The backup is the schema 1 file with the same rows.
    let copy = Connection::open(&backup).unwrap();
    assert_eq!(migrate::user_version(&copy).unwrap(), 1);
    let n: i64 = copy
        .query_row("SELECT count(*) FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

/// A set-aside copy as `new_vault` leaves one after a clean close (no
/// `-wal` beside it): bound or not, with `kdf` as its settings' text.
fn set_aside_copy(dir: &Path, name: &str, bound: bool, kdf: &str) {
    let conn = Connection::open(dir.join(name)).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE server_account (id INTEGER PRIMARY KEY);
         CREATE TABLE vaults (vault_id BLOB, account_user_id TEXT);
         CREATE TABLE local_auth (id INTEGER PRIMARY KEY, kdf TEXT NOT NULL, kdf_salt BLOB NOT NULL);",
    )
    .unwrap();
    if bound {
        conn.execute("INSERT INTO server_account (id) VALUES (1)", [])
            .unwrap();
    }
    conn.execute(
        "INSERT INTO local_auth (id, kdf, kdf_salt) VALUES (1, ?1, x'00')",
        [kdf],
    )
    .unwrap();
}

/// Run 13 (D217, its review): the newest bound copy gives the reference,
/// by stamp and number (`-2` after the first, `-10` after `-2`); an
/// unbound, a half-made (`-failed`) or a user-named copy does not count;
/// reading leaves no `-wal` or `-shm` beside a copy.
#[test]
fn the_newest_bound_set_aside_copy_gives_the_settings_and_leaves_nothing_beside_it() {
    let dir = TempDir::new().unwrap();
    for (name, bound, kdf) in [
        ("shellpp.db.20261020T101500Z.old", true, "oldest"),
        ("shellpp.db.20261021T000000Z.old", true, "first"),
        ("shellpp.db.20261021T000000Z-2.old", true, "second"),
        ("shellpp.db.20261021T000000Z-10.old", false, "unbound"),
        ("shellpp.db.20261022T000000Z-failed.old", true, "failed"),
        ("shellpp.db.mine.old", true, "mine"),
    ] {
        set_aside_copy(dir.path(), name, bound, kdf);
    }
    let (kdf, salt) = set_aside_bound_kdf(dir.path()).expect("a bound copy");
    assert_eq!((kdf.as_str(), salt), ("second", vec![0]));
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names
            .iter()
            .all(|name| !name.ends_with("-wal") && !name.ends_with("-shm")),
        "{names:?}"
    );
}

/// A path with a space, `%`, `#` and a letter outside ASCII reaches the
/// copy through its `file:` URI.
#[test]
fn a_set_aside_copy_is_read_under_a_path_with_unusual_characters() {
    let root = TempDir::new().unwrap();
    let dir = root.path().join("a b%#\u{e9}");
    std::fs::create_dir(&dir).unwrap();
    set_aside_copy(&dir, "shellpp.db.20261021T000000Z.old", true, "found");
    let (kdf, _) = set_aside_bound_kdf(&dir).expect("a bound copy");
    assert_eq!(kdf, "found");
}

/// A copy set aside with its `-wal` (the rows only in the log) is read
/// with it, not as an immutable file without them.
#[test]
fn a_set_aside_copy_with_its_wal_is_read_with_it() {
    let dir = TempDir::new().unwrap();
    let live = dir.path().join("live.db");
    let conn = Connection::open(&live).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA wal_autocheckpoint = 0;
         CREATE TABLE server_account (id INTEGER PRIMARY KEY);
         CREATE TABLE vaults (vault_id BLOB, account_user_id TEXT);
         CREATE TABLE local_auth (id INTEGER PRIMARY KEY, kdf TEXT NOT NULL, kdf_salt BLOB NOT NULL);
         INSERT INTO server_account (id) VALUES (1);
         INSERT INTO local_auth (id, kdf, kdf_salt) VALUES (1, 'in the log', x'00');",
    )
    .unwrap();
    let name = "shellpp.db.20261021T000000Z.old";
    std::fs::copy(&live, dir.path().join(name)).unwrap();
    std::fs::copy(
        dir.path().join("live.db-wal"),
        dir.path().join(format!("{name}-wal")),
    )
    .unwrap();
    drop(conn);
    let (kdf, _) = set_aside_bound_kdf(dir.path()).expect("a bound copy");
    assert_eq!(kdf, "in the log");
}
