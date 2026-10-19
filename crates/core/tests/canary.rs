//! Canary tests (docs/security.md, section 7): secrets and host data pass
//! through the code paths with a TRACE-level `tracing` subscriber capturing
//! every log line; no canary may appear in the log output, in `Debug` output,
//! or in plaintext in `shellpp.db` and `shellpp.db-wal`.

mod common;

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use common::{Env, as_host, save};
use data_encoding::BASE64URL_NOPAD;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use shellpp_core::crypto::decode_recovery_key;
use shellpp_core::service::dto::{
    QueryScope, RecordRef, RecordType, RecordsQuery, RecordsRef, SecretCopyTarget, SecretFieldRef,
    SortMode,
};
use tracing_subscriber::fmt::MakeWriter;

/// The tests of this file run one at a time: `tracing` caches whether a log
/// call site is enabled, and a test without a subscriber that reaches a call
/// site while another thread sets up its scoped subscriber can leave it
/// cached as disabled, so the capturing test would miss log lines.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Every canary absent from the file (missing files count as clean).
fn assert_file_clean(path: &Path, canaries: &[(&str, Vec<u8>)]) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    for (name, canary) in canaries {
        assert!(
            !contains(&bytes, canary),
            "canary {name} found in plaintext in {}",
            path.file_name().unwrap().to_string_lossy()
        );
    }
}

/// The `wrap` blob of the current-epoch key wrap of this kind, read through a
/// separate read-only connection that is closed again before the core writes.
fn wrap_blob(data_dir: &Path, kind: &str) -> Vec<u8> {
    let conn = rusqlite::Connection::open_with_flags(
        data_dir.join("shellpp.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("read-only connection");
    conn.query_row(
        "SELECT w.wrap FROM key_wraps w
         JOIN vaults v ON v.vault_id = w.vault_id AND v.current_epoch = w.epoch
         WHERE w.kind = ?1",
        [kind],
        |row| row.get(0),
    )
    .expect("key wrap row")
}

/// Wraps replaced by a credential change are gone from the database file and
/// the WAL: with a copy of the data directory, the old password or the old
/// recovery key must not open the vault key.
fn assert_wraps_gone(data_dir: &Path, wraps: &[(&str, Vec<u8>)]) {
    assert_file_clean(&data_dir.join("shellpp.db"), wraps);
    assert_file_clean(&data_dir.join("shellpp.db-wal"), wraps);
}

#[test]
fn replaced_key_wraps_do_not_stay_in_the_database_files() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let password = common::new_password();
    core.first_run_setup(password.clone()).expect("first run");
    let data_dir = env.config().data_dir;

    // Reissue: the old recovery key stops working.
    let old = [
        ("previous recovery wrap", wrap_blob(&data_dir, "recovery")),
        ("previous kek wrap", wrap_blob(&data_dir, "kek")),
    ];
    let reissued = core.recovery_key_reissue(password).expect("reissue");
    assert_ne!(wrap_blob(&data_dir, "recovery"), old[0].1);
    assert_wraps_gone(&data_dir, &old);

    // Recovery unlock: a new master password replaces the old one.
    core.lock().unwrap();
    let old = [
        ("previous recovery wrap", wrap_blob(&data_dir, "recovery")),
        ("previous kek wrap", wrap_blob(&data_dir, "kek")),
    ];
    core.recovery_unlock(
        SecretString::from(reissued.recovery_key.expose_secret().to_owned()),
        common::new_password(),
    )
    .expect("recovery unlock");
    assert!(core.is_unlocked());
    assert_wraps_gone(&data_dir, &old);
}

#[test]
fn canaries_never_reach_logs_debug_output_or_disk() {
    let _serial = serial();
    let tag = common::random_hex();
    let master = format!("CanaryMaster-{tag}");
    let master_2 = format!("CanaryMasterTwo-{tag}");
    let wrong = format!("CanaryWrong-{tag}");
    let host_password = format!("CanaryHostPass-{tag}");
    let address = format!("canary-{tag}.example.com");
    let username = format!("canaryuser{tag}");
    let label = format!("CanaryLabel-{tag}");
    let notes = format!("CanaryNotes-{tag}");
    let env_value = format!("CanaryEnv-{tag}");
    let search = format!("canary-{tag}");

    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .finish();

    let env = Env::new();
    let mut debug_output = String::new();
    let mut recovery_text = String::new();
    let mut files_checked = false;
    tracing::subscriber::with_default(subscriber, || {
        let core = env.open();
        let recovery = core
            .first_run_setup(SecretString::from(master.clone()))
            .expect("first run");
        recovery_text = recovery.recovery_key.expose_secret().to_owned();
        debug_output.push_str(&format!("{recovery:?}"));
        let vault_id = core.vaults().unwrap()[0].vault_id;

        let host = as_host(
            save(
                &core,
                vault_id,
                None,
                RecordType::Host,
                json!({
                    "address": address,
                    "label": label,
                    "notes": notes,
                    "ssh": {"username": username, "env": [{"name": "CANARY", "value": env_value}]}
                }),
                &[("ssh.password", Some(&host_password))],
            )
            .expect("host"),
        );
        debug_output.push_str(&format!("{host:?}"));
        // Validation errors mention fields, not values.
        let error = save(
            &core,
            vault_id,
            Some(host.record_id),
            RecordType::Host,
            json!({"address": format!("{address} bad"), "ssh": {"username": username}}),
            &[("ssh.password", Some(&host_password))],
        )
        .unwrap_err();
        debug_output.push_str(&format!("{error:?} {error} {}", error.details()));

        let query = RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host, RecordType::Group],
            scope: QueryScope::All,
            text: Some(search.clone()),
            tags: vec![],
            sort: SortMode::Manual,
        };
        debug_output.push_str(&format!("{query:?}"));
        let found = core.records_query(query).unwrap();
        assert_eq!(found.len(), 1);
        debug_output.push_str(&format!("{found:?}"));
        let revealed = core
            .secret_reveal(SecretFieldRef {
                vault_id,
                record_id: host.record_id,
                path: "ssh.password".into(),
            })
            .unwrap();
        assert_eq!(revealed.expose_secret(), host_password);
        debug_output.push_str(&format!("{revealed:?}"));
        core.secret_copy(SecretCopyTarget::RecordField {
            vault_id,
            record_id: host.record_id,
            path: "ssh.password".into(),
        })
        .unwrap();
        core.secret_copy(SecretCopyTarget::RecoveryKey).unwrap();
        let duplicate = core
            .record_duplicate(RecordRef {
                vault_id,
                record_id: host.record_id,
            })
            .unwrap();
        core.record_delete(RecordsRef {
            vault_id,
            record_ids: vec![duplicate.record_id()],
        })
        .unwrap();
        core.lock().unwrap();
        assert!(core.unlock(SecretString::from(wrong.clone())).is_err());
        core.unlock(SecretString::from(master.clone())).unwrap();
        let data_dir = env.config().data_dir;
        let old_kek = wrap_blob(&data_dir, "kek");
        core.password_change(
            SecretString::from(master.clone()),
            SecretString::from(master_2.clone()),
        )
        .unwrap();
        // The previous `kek` wrap opens the vault key with the old password.
        assert_ne!(wrap_blob(&data_dir, "kek"), old_kek);
        assert_wraps_gone(&data_dir, &[("previous kek wrap", old_kek)]);
        core.recovery_key_show(SecretString::from(master_2.clone()))
            .unwrap();
        debug_output.push_str(&format!("{core:?} {:?}", core.snapshot()));

        // Files on disk while the database is open (the WAL exists now).
        let key = decode_recovery_key(&recovery_text).unwrap();
        let recovery_b64 = BASE64URL_NOPAD.encode(key.expose());
        let canaries: Vec<(&str, Vec<u8>)> = vec![
            ("master password", master.clone().into_bytes()),
            ("second master password", master_2.clone().into_bytes()),
            ("host password", host_password.clone().into_bytes()),
            ("address", address.clone().into_bytes()),
            ("username", username.clone().into_bytes()),
            ("label", label.clone().into_bytes()),
            ("notes", notes.clone().into_bytes()),
            ("env value", env_value.clone().into_bytes()),
            ("recovery key text", recovery_text.clone().into_bytes()),
            ("recovery key b64u", recovery_b64.into_bytes()),
            ("recovery key bytes", key.expose().to_vec()),
        ];
        assert!(data_dir.join("shellpp.db").exists());
        assert_file_clean(&data_dir.join("shellpp.db"), &canaries);
        assert_file_clean(&data_dir.join("shellpp.db-wal"), &canaries);
        assert_file_clean(&data_dir.join("shellpp.db-shm"), &canaries);
        core.on_exit();
        drop(core);
        assert_file_clean(&data_dir.join("shellpp.db"), &canaries);
        assert_file_clean(&data_dir.join("shellpp.db-wal"), &canaries);
        files_checked = true;
    });
    assert!(files_checked);

    let log = capture.text();
    assert!(
        log.contains("first run completed"),
        "the subscriber captured the core's logs"
    );
    assert!(log.contains("unlocked"));
    for (name, canary) in [
        ("master password", master.as_str()),
        ("second master password", master_2.as_str()),
        ("wrong password", wrong.as_str()),
        ("host password", host_password.as_str()),
        ("address", address.as_str()),
        ("username", username.as_str()),
        ("label", label.as_str()),
        ("notes", notes.as_str()),
        ("env value", env_value.as_str()),
        ("search text", search.as_str()),
        ("recovery key", recovery_text.as_str()),
    ] {
        assert!(
            !log.contains(canary),
            "canary {name} found in the log output"
        );
        assert!(
            !debug_output.contains(canary),
            "canary {name} found in Debug output"
        );
    }
}
