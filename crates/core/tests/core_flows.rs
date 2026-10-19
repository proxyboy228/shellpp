//! Lock state machine flows through the facade with a temporary data
//! directory (docs/security.md, sections 5.9 and 8; UI 13.1-13.4).
//!
//! Each Argon2id call runs at the real desktop preset (no weakened test
//! parameters), so the tests keep the number of password derivations small.

mod common;

use std::time::Duration;

use common::{Env, first_run, names, new_password};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use shellpp_core::crypto::{Key32, encode_recovery_key};
use shellpp_core::service::dto::{
    CoreEventKind, LockState, QueryScope, RecordRef, RecordType, RecordsQuery, SecretCopyTarget,
    SecretFieldRef, SettingsSetInput, SortMode,
};
use shellpp_core::store::{Store, WrapKind};
use shellpp_core::{CoreError, ValidationReason};

fn reveal(core: &shellpp_core::Core, vault_id: uuid::Uuid, record_id: uuid::Uuid) -> String {
    core.secret_reveal(SecretFieldRef {
        vault_id,
        record_id,
        path: "ssh.password".into(),
    })
    .expect("reveal")
    .expose_secret()
    .to_owned()
}

fn all_hosts(
    core: &shellpp_core::Core,
) -> Result<Vec<shellpp_core::service::dto::RecordView>, CoreError> {
    core.records_query(RecordsQuery {
        vault_id: None,
        types: vec![RecordType::Host],
        scope: QueryScope::All,
        text: None,
        tags: Vec::new(),
        sort: SortMode::Manual,
    })
}

fn set_setting(core: &shellpp_core::Core, key: &str, value: serde_json::Value) {
    let mut values = std::collections::BTreeMap::new();
    values.insert(key.to_owned(), value);
    core.settings_set(SettingsSetInput {
        values,
        reset: Vec::new(),
    })
    .expect("settings_set");
}

#[test]
fn first_run_lock_unlock_round_trip() {
    let env = Env::new();
    let core = env.open();
    let status = core.lock_status();
    assert_eq!(status.state, LockState::FirstRun);
    assert!(status.remember_device_available);
    assert!(!status.remember_device_enabled);
    assert_eq!(core.snapshot().v, 0);
    assert!(core.snapshot().vaults.is_empty());

    // Guards at first run.
    assert!(matches!(core.lock(), Err(CoreError::WrongState)));
    assert!(matches!(
        core.unlock(new_password()),
        Err(CoreError::WrongState)
    ));
    assert!(matches!(all_hosts(&core), Err(CoreError::Locked)));
    assert!(matches!(core.vaults(), Err(CoreError::Locked)));
    let short = core
        .first_run_setup(SecretString::from("1234567"))
        .unwrap_err();
    assert_eq!(
        common::reasons(short),
        vec![("password".to_owned(), ValidationReason::TooShort)]
    );
    assert!(core.settings_get().is_ok());

    let password = new_password();
    let recovery = core.first_run_setup(password.clone()).expect("first run");
    let text = recovery.recovery_key.expose_secret().to_owned();
    assert!(text.starts_with("RK1-"));
    assert_eq!(text.split('-').count(), 12);
    assert!(text.split('-').skip(1).all(|group| group.len() == 5));
    assert!(core.is_unlocked());
    let events = env.take_events();
    assert_eq!(names(&events), vec!["lock:changed", "vaults:changed"]);
    assert_eq!(events[0].v, 1);
    assert_eq!(events[1].v, 2);
    assert!(matches!(
        events[0].kind,
        CoreEventKind::LockChanged {
            locked: false,
            state: LockState::Unlocked
        }
    ));
    let snapshot = core.snapshot();
    assert_eq!(snapshot.v, 2);
    assert_eq!(snapshot.vaults.len(), 1);
    assert_eq!(snapshot.vaults[0].name, "Personal");
    let vault_id = snapshot.vaults[0].vault_id;
    assert!(matches!(
        core.first_run_setup(new_password()),
        Err(CoreError::WrongState)
    ));

    // A host, then lock.
    let host = common::save(
        &core,
        vault_id,
        None,
        RecordType::Host,
        json!({"address": "db1.example.com", "label": "DB"}),
        &[("ssh.password", Some("hunter-two-xyz"))],
    )
    .expect("host");
    let host_id = host.record_id();
    let status = core.lock().expect("lock");
    assert_eq!(status.state, LockState::Locked);
    assert!(matches!(core.lock(), Ok(status) if status.state == LockState::Locked));
    assert!(matches!(all_hosts(&core), Err(CoreError::Locked)));
    assert!(matches!(
        core.record_get(RecordRef {
            vault_id,
            record_id: host_id
        }),
        Err(CoreError::Locked)
    ));
    assert!(matches!(core.vaults(), Err(CoreError::Locked)));
    assert!(core.snapshot().vaults.is_empty());
    assert!(matches!(
        core.settings_set(SettingsSetInput::default()),
        Err(CoreError::Locked)
    ));
    assert!(matches!(
        core.first_run_setup(new_password()),
        Err(CoreError::WrongState)
    ));

    // Wrong password: nothing changes.
    assert!(matches!(
        core.unlock(new_password()),
        Err(CoreError::InvalidPassword)
    ));
    assert_eq!(core.lock_status().state, LockState::Locked);

    let status = core.unlock(password.clone()).expect("unlock");
    assert_eq!(status.state, LockState::Unlocked);
    assert!(matches!(
        core.unlock(password.clone()),
        Err(CoreError::WrongState)
    ));
    assert_eq!(common::ids(&all_hosts(&core).unwrap()), vec![host_id]);
    assert_eq!(reveal(&core, vault_id, host_id), "hunter-two-xyz");

    // Events are strictly increasing and the snapshot sees the last one.
    let events = env.take_events();
    let versions: Vec<u64> = events.iter().map(|event| event.v).collect();
    assert!(versions.windows(2).all(|pair| pair[1] == pair[0] + 1));
    assert_eq!(core.snapshot().v, *versions.last().unwrap());
    assert!(names(&events).contains(&"records:changed"));
    assert_eq!(
        names(&events)
            .iter()
            .filter(|name| **name == "lock:changed")
            .count(),
        2
    );

    // Restart: the app starts locked; the same password opens it.
    drop(core);
    let core = env.open();
    assert_eq!(core.lock_status().state, LockState::Locked);
    assert_eq!(core.snapshot().v, 0);
    core.unlock(password).expect("unlock after restart");
    assert_eq!(common::ids(&all_hosts(&core).unwrap()), vec![host_id]);
}

#[test]
fn recovery_unlock_rotates_the_vault_and_replaces_the_password() {
    let mut app = first_run();
    let host = app.host_with_password(
        json!({"address": "198.51.100.20"}),
        "secret-before-recovery",
    );
    app.core.lock().unwrap();

    // Malformed input is reported per field without touching anything.
    let errors = common::reasons(
        app.core
            .recovery_unlock(SecretString::from("RK1-NOPE"), SecretString::from("short"))
            .unwrap_err(),
    );
    assert_eq!(
        errors,
        vec![
            ("recovery_key".to_owned(), ValidationReason::Invalid),
            ("new_password".to_owned(), ValidationReason::TooShort),
        ]
    );
    // A well-formed key that is not ours.
    let foreign = encode_recovery_key(&Key32::generate().unwrap());
    assert!(matches!(
        app.core.recovery_unlock(foreign, new_password()),
        Err(CoreError::InvalidRecoveryKey)
    ));

    // Lowercase and spaces are accepted.
    let typed = app.recovery.to_lowercase().replace('-', " ");
    let new = new_password();
    let status = app
        .core
        .recovery_unlock(SecretString::from(typed), new.clone())
        .expect("recovery unlock");
    assert_eq!(status.state, LockState::Unlocked);
    let events = app.env.take_events();
    assert!(names(&events).contains(&"vaults:changed"));
    assert!(names(&events).contains(&"records:changed"));
    assert!(names(&events).contains(&"lock:changed"));
    assert_eq!(
        reveal(&app.core, app.vault_id, host.record_id),
        "secret-before-recovery"
    );
    // The re-seal wrote a new revision of every record.
    let view = app
        .core
        .record_get(RecordRef {
            vault_id: app.vault_id,
            record_id: host.record_id,
        })
        .unwrap();
    assert_eq!(common::as_host(view).revision, 2);

    // The old password no longer opens the vault; the new one does.
    app.core.lock().unwrap();
    assert!(matches!(
        app.core.unlock(app.password.clone()),
        Err(CoreError::InvalidPassword)
    ));
    app.core.unlock(new.clone()).expect("new password");

    // The recovery key itself stays valid (local mode keeps it).
    app.core.lock().unwrap();
    app.core
        .recovery_unlock(SecretString::from(app.recovery.clone()), new_password())
        .expect("second recovery");
    assert_eq!(
        reveal(&app.core, app.vault_id, host.record_id),
        "secret-before-recovery"
    );

    // Storage: epoch 3, only current-epoch kek and recovery rows, chain rows 2 and 3.
    let config = app.env.config();
    drop(std::mem::replace(&mut app.core, app.env.open()));
    let (store, _) = Store::open(&config.data_dir, common::APP_VERSION).unwrap();
    let vault = store.vault(&app.vault_id).unwrap().unwrap();
    assert_eq!(vault.current_epoch, 3);
    let wraps: Vec<(u32, WrapKind)> = store
        .key_wraps(&app.vault_id)
        .unwrap()
        .into_iter()
        .map(|row| (row.epoch, row.kind))
        .collect();
    assert!(wraps.contains(&(3, WrapKind::Kek)));
    assert!(wraps.contains(&(3, WrapKind::Recovery)));
    assert!(wraps.contains(&(2, WrapKind::Chain)));
    assert!(wraps.contains(&(3, WrapKind::Chain)));
    assert!(
        !wraps
            .iter()
            .any(|(epoch, kind)| *epoch < 3 && matches!(kind, WrapKind::Kek | WrapKind::Recovery))
    );
    let rows = store.records(&app.vault_id, None).unwrap();
    assert!(rows.iter().all(|row| row.key_epoch == 3));
}

#[test]
fn password_change_rewraps_without_reencryption() {
    let app = first_run();
    let host = app.host(json!({"address": "web.example.org"}));
    let short = app
        .core
        .password_change(app.password.clone(), SecretString::from("tiny"))
        .unwrap_err();
    assert_eq!(
        common::reasons(short),
        vec![("new_password".to_owned(), ValidationReason::TooShort)]
    );
    assert!(matches!(
        app.core.password_change(new_password(), new_password()),
        Err(CoreError::InvalidPassword)
    ));
    let new = new_password();
    app.core
        .password_change(app.password.clone(), new.clone())
        .expect("password change");
    // No record was re-sealed.
    let view = app
        .core
        .record_get(RecordRef {
            vault_id: app.vault_id,
            record_id: host.record_id,
        })
        .unwrap();
    assert_eq!(common::as_host(view).revision, 1);
    app.core.lock().unwrap();
    assert!(matches!(
        app.core.unlock(app.password.clone()),
        Err(CoreError::InvalidPassword)
    ));
    app.core.unlock(new).expect("unlock with the new password");
    // The recovery key still works after a password change.
    app.core.lock().unwrap();
    app.core
        .recovery_unlock(SecretString::from(app.recovery.clone()), new_password())
        .expect("recovery after password change");

    // Locked: password change needs the unlocked state.
    app.core.lock().unwrap();
    assert!(matches!(
        app.core.password_change(new_password(), new_password()),
        Err(CoreError::Locked)
    ));
}

#[test]
fn recovery_key_show_copy_and_reissue() {
    let app = first_run();
    let core = &app.core;
    // The first run grants a short copy window.
    core.secret_copy(SecretCopyTarget::RecoveryKey)
        .expect("copy after first run");
    assert_eq!(
        app.env.mem.clipboard.text().as_deref(),
        Some(app.recovery.as_str())
    );
    assert!(app.env.mem.clipboard.last_was_sensitive());
    app.env.mem.clock.advance(Duration::from_secs(5 * 60));
    assert!(matches!(
        core.secret_copy(SecretCopyTarget::RecoveryKey),
        Err(CoreError::WrongState)
    ));

    assert!(matches!(
        core.recovery_key_show(new_password()),
        Err(CoreError::InvalidPassword)
    ));
    let shown = core.recovery_key_show(app.password.clone()).expect("show");
    assert_eq!(shown.recovery_key.expose_secret(), app.recovery);
    core.secret_copy(SecretCopyTarget::RecoveryKey)
        .expect("copy after show");

    let reissued = core
        .recovery_key_reissue(app.password.clone())
        .expect("reissue");
    let new_key = reissued.recovery_key.expose_secret().to_owned();
    assert_ne!(new_key, app.recovery);
    let events = app.env.take_events();
    assert!(names(&events).contains(&"vaults:changed"));
    assert!(names(&events).contains(&"records:changed"));
    let shown = core
        .recovery_key_show(app.password.clone())
        .expect("show again");
    assert_eq!(shown.recovery_key.expose_secret(), new_key);

    // The password is unchanged by a reissue.
    core.lock().unwrap();
    assert!(matches!(
        core.secret_copy(SecretCopyTarget::RecoveryKey),
        Err(CoreError::Locked)
    ));
    // The old key no longer opens anything current.
    assert!(matches!(
        core.recovery_unlock(SecretString::from(app.recovery.clone()), new_password()),
        Err(CoreError::InvalidRecoveryKey)
    ));
    core.recovery_unlock(SecretString::from(new_key), new_password())
        .expect("new key works");
    // The grant does not survive a lock.
    assert!(matches!(
        core.secret_copy(SecretCopyTarget::RecoveryKey),
        Err(CoreError::WrongState)
    ));
}

#[test]
fn remember_on_this_device() {
    let mut app = first_run();
    let status = app.core.remember_device_set(true).expect("remember on");
    assert!(status.remember_device_enabled);
    let accounts = app.env.mem.secret_store.accounts();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].starts_with("device-key:"));

    // Launch: unlocked with the device key, no password.
    app.reopen();
    assert_eq!(app.core.lock_status().state, LockState::Locked);
    assert!(app.core.lock_status().remember_device_enabled);
    assert!(app.core.try_device_unlock().expect("device unlock"));
    assert!(app.core.is_unlocked());
    // Never after a lock in the same process.
    app.core.lock().unwrap();
    assert!(!app.core.try_device_unlock().unwrap());
    app.core
        .unlock(app.password.clone())
        .expect("password still works");

    // A rotation keeps the device wrap for the new epoch.
    app.core
        .recovery_key_reissue(app.password.clone())
        .expect("reissue");
    assert!(app.core.lock_status().remember_device_enabled);
    app.reopen();
    assert!(app.core.try_device_unlock().unwrap());

    // Off: keychain item and wrap deleted.
    let status = app.core.remember_device_set(false).expect("remember off");
    assert!(!status.remember_device_enabled);
    assert!(app.env.mem.secret_store.accounts().is_empty());
    app.reopen();
    assert!(!app.core.try_device_unlock().unwrap());

    // Without an OS keychain the option is unavailable.
    app.env.mem.secret_store.set_available(false);
    assert!(!app.core.lock_status().remember_device_available);
    app.core.unlock(app.password.clone()).unwrap();
    assert!(matches!(
        app.core.remember_device_set(true),
        Err(CoreError::Unavailable)
    ));
}

#[test]
fn remember_toggle_keeps_an_existing_key_and_notifies_every_window() {
    use shellpp_core::platform::SecretStore;

    let app = first_run();
    let store = &app.env.mem.secret_store;
    app.core.remember_device_set(true).expect("remember on");
    assert_eq!(names(&app.env.take_events()), vec!["vaults:changed"]);
    let account = store.accounts()[0].clone();
    let key = store.get(&account).unwrap().expect("device key");

    // A window with a stale toggle turns it on again: same key, no event.
    let status = app.core.remember_device_set(true).expect("still on");
    assert!(status.remember_device_enabled);
    let again = store.get(&account).unwrap().expect("device key");
    assert_eq!(again.expose_secret(), key.expose_secret());
    assert!(app.env.take_events().is_empty());

    app.core.remember_device_set(false).expect("remember off");
    assert_eq!(names(&app.env.take_events()), vec!["vaults:changed"]);
    assert!(store.accounts().is_empty());
}

#[test]
fn first_run_never_replaces_a_completed_first_run() {
    // Two cores on one data directory, as two processes would be.
    let env = Env::new();
    let first = env.open();
    let second = env.open();
    let password = new_password();
    first.first_run_setup(password.clone()).expect("first run");
    assert!(matches!(
        second.first_run_setup(new_password()),
        Err(CoreError::WrongState)
    ));
    drop(second);
    drop(first);
    let core = env.open();
    core.unlock(password)
        .expect("the first password still works");
}

#[test]
fn rows_with_unreadable_columns_do_not_fail_the_unlock() {
    let app = first_run();
    let good = app.host(json!({"address": "good.example.com"}));
    let bad = app.host(json!({"address": "bad.example.com"}));
    app.core.lock().unwrap();
    let conn =
        rusqlite::Connection::open(app.env.config().data_dir.join("shellpp.db")).expect("db");
    conn.execute(
        "UPDATE records SET revision = -1 WHERE record_id = ?1",
        [&bad.record_id.as_bytes()[..]],
    )
    .expect("update");
    drop(conn);
    app.core
        .unlock(app.password.clone())
        .expect("unlock with an unreadable row");
    assert_eq!(
        common::ids(&all_hosts(&app.core).unwrap()),
        vec![good.record_id]
    );
}

#[test]
fn auto_lock_follows_the_wall_clock() {
    let app = first_run();
    let core = &app.core;
    let clock = &app.env.mem.clock;
    set_setting(core, "security.auto_lock_minutes", json!(1));
    core.activity();
    clock.advance(Duration::from_secs(59));
    assert!(!core.auto_lock_due());
    core.tick();
    assert!(core.is_unlocked());
    core.activity();
    clock.advance(Duration::from_secs(50));
    core.tick();
    assert!(core.is_unlocked(), "activity restarted the timer");
    // A clock set backwards restarts the timer instead of locking.
    clock.rewind(Duration::from_secs(3600));
    core.tick();
    assert!(core.is_unlocked());
    clock.advance(Duration::from_secs(61));
    app.env.take_events();
    // The check alone does not lock: a host saves pending edits first.
    assert!(core.auto_lock_due());
    assert!(core.is_unlocked());
    core.tick();
    assert!(!core.is_unlocked());
    assert!(!core.auto_lock_due(), "never due while locked");
    let events = app.env.take_events();
    assert!(matches!(
        events[0].kind,
        CoreEventKind::LockChanged {
            locked: true,
            state: LockState::Locked
        }
    ));

    core.unlock(app.password.clone()).unwrap();
    set_setting(core, "security.auto_lock_minutes", serde_json::Value::Null);
    clock.advance(Duration::from_secs(2 * 3600));
    core.tick();
    assert!(core.is_unlocked(), "Never disables auto-lock");
}

#[test]
fn input_after_a_long_sleep_locks_instead_of_restarting_the_timer() {
    let app = first_run();
    let core = &app.core;
    let clock = &app.env.mem.clock;
    set_setting(core, "security.auto_lock_minutes", json!(1));
    core.activity();
    // The machine sleeps past the interval; the first input reaches the core
    // before the next tick.
    clock.advance(Duration::from_secs(3 * 3600));
    app.env.take_events();
    core.activity();
    assert_eq!(core.lock_status().state, LockState::Locked);
    assert!(matches!(
        app.env.take_events()[0].kind,
        CoreEventKind::LockChanged {
            locked: true,
            state: LockState::Locked
        }
    ));
    core.tick();
    assert_eq!(core.lock_status().state, LockState::Locked);
    // Input while locked changes nothing; input before the interval still
    // restarts the timer.
    core.activity();
    core.unlock(app.password.clone()).unwrap();
    clock.advance(Duration::from_secs(59));
    core.activity();
    clock.advance(Duration::from_secs(59));
    core.tick();
    assert!(core.is_unlocked());
}

#[test]
fn sensitive_clipboard_is_cleared_only_when_unchanged() {
    let app = first_run();
    let core = &app.core;
    let clock = &app.env.mem.clock;
    let clipboard = &app.env.mem.clipboard;
    let host = app.host_with_password(json!({"address": "203.0.113.9"}), "copy-me-please-123");
    let target = || SecretCopyTarget::RecordField {
        vault_id: app.vault_id,
        record_id: host.record_id,
        path: "ssh.password".into(),
    };
    core.secret_copy(target()).unwrap();
    assert_eq!(clipboard.text().as_deref(), Some("copy-me-please-123"));
    assert!(clipboard.last_was_sensitive());
    clock.advance(Duration::from_secs(29));
    core.tick();
    assert!(clipboard.text().is_some());
    clock.advance(Duration::from_secs(2));
    core.tick();
    assert_eq!(clipboard.text(), None);

    // The user copied something else meanwhile: it stays.
    core.secret_copy(target()).unwrap();
    clipboard.set_text(Some("user text"));
    clock.advance(Duration::from_secs(31));
    core.tick();
    assert_eq!(clipboard.text().as_deref(), Some("user text"));

    // Lock and exit clear it too.
    core.secret_copy(target()).unwrap();
    core.lock().unwrap();
    assert_eq!(clipboard.text(), None);
    core.unlock(app.password.clone()).unwrap();
    core.secret_copy(target()).unwrap();
    core.on_exit();
    assert_eq!(clipboard.text(), None);

    // Errors.
    assert!(matches!(
        core.secret_copy(SecretCopyTarget::RecordField {
            vault_id: app.vault_id,
            record_id: host.record_id,
            path: "address".into(),
        }),
        Err(CoreError::Validation(_))
    ));
    clipboard.set_available(false);
    assert!(matches!(
        core.secret_copy(target()),
        Err(CoreError::Unavailable)
    ));
}

#[test]
fn clipboard_clearing_is_retried_after_a_transient_error() {
    let app = first_run();
    let core = &app.core;
    let clock = &app.env.mem.clock;
    let clipboard = &app.env.mem.clipboard;
    let host = app.host_with_password(json!({"address": "203.0.113.10"}), "retry-me-please-42");
    let target = || SecretCopyTarget::RecordField {
        vault_id: app.vault_id,
        record_id: host.record_id,
        path: "ssh.password".into(),
    };

    // Another program holds the clipboard open when the 30 seconds pass.
    core.secret_copy(target()).unwrap();
    clipboard.set_available(false);
    clock.advance(Duration::from_secs(31));
    core.tick();
    clipboard.set_available(true);
    assert_eq!(clipboard.text().as_deref(), Some("retry-me-please-42"));
    clock.advance(Duration::from_secs(1));
    core.tick();
    assert_eq!(clipboard.text(), None, "the next tick retried");

    // The same on lock, before the 30 seconds: the next tick retries.
    core.secret_copy(target()).unwrap();
    clipboard.set_available(false);
    core.lock().unwrap();
    clipboard.set_available(true);
    assert!(clipboard.text().is_some());
    clock.advance(Duration::from_secs(1));
    core.tick();
    assert_eq!(clipboard.text(), None, "retried while locked");

    // Retries stop after five minutes.
    core.unlock(app.password.clone()).unwrap();
    core.secret_copy(target()).unwrap();
    clipboard.set_available(false);
    for _ in 0..7 {
        clock.advance(Duration::from_secs(60));
        core.tick();
    }
    clipboard.set_available(true);
    clock.advance(Duration::from_secs(1));
    core.tick();
    assert!(clipboard.text().is_some(), "given up");
}

#[test]
fn damaged_rows_are_kept_out_of_the_index_and_app_version_is_updated() {
    let mut app = first_run();
    let good = app.host(json!({"address": "good.example.com"}));
    let bad = app.host(json!({"address": "bad.example.com"}));
    let config = app.env.config();
    drop(std::mem::replace(&mut app.core, app.env.open()));

    // Flip one ciphertext byte of one row (header and columns stay consistent).
    {
        let (mut store, _) = Store::open(&config.data_dir, common::APP_VERSION).unwrap();
        let mut row = store
            .record(&app.vault_id, &bad.record_id)
            .unwrap()
            .unwrap();
        let last = row.envelope.len() - 1;
        row.envelope[last] ^= 0x01;
        store.write(|tx| tx.record_put(&row)).unwrap();
    }

    // A newer app version opens the database: `device_info` is rewritten on unlock.
    let mut newer = config.clone();
    newer.app_version = "0.2.0".to_owned();
    let core = shellpp_core::Core::open(
        newer,
        app.env
            .mem
            .platform(shellpp_core::platform::OsKind::Windows, "TEST-PC"),
    )
    .unwrap();
    app.env.take_events();
    core.unlock(app.password.clone())
        .expect("unlock with a damaged row");
    let hosts = common::ids(&all_hosts(&core).unwrap());
    assert_eq!(hosts, vec![good.record_id]);
    let events = app.env.take_events();
    assert_eq!(names(&events), vec!["records:changed", "lock:changed"]);
    // The damaged row is still stored, unchanged.
    drop(core);
    let (store, _) = Store::open(&config.data_dir, common::APP_VERSION).unwrap();
    assert!(
        store
            .record(&app.vault_id, &bad.record_id)
            .unwrap()
            .is_some()
    );
}
