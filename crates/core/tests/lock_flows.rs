//! Lock flows of decision D29 (owner overrides, UI 13.1-13.3) through the
//! facade with a temporary data directory and the in-memory platform (never
//! the OS keychain): the 8-character minimum without composition rules,
//! "Remember on this device" on by default, the unlock attempt limit that
//! survives a restart, the auto-lock default "Never", and "Start with a new
//! vault", which renames the data file and never deletes it.
//!
//! Each Argon2id call runs at the real desktop preset, so the tests keep the
//! number of password derivations small.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Env, names, new_password};
use secrecy::SecretString;
use serde_json::json;
use shellpp_core::lock::FirstRunOptions;
use shellpp_core::lock::lockout::META_UNLOCK_LOCKOUT;
use shellpp_core::service::dto::{LockState, QueryScope, RecordType, RecordsQuery, SortMode};
use shellpp_core::store::Store;
use shellpp_core::{CoreError, ValidationReason};
use tracing_subscriber::fmt::MakeWriter;

/// The tests of this file run one at a time: `tracing` caches whether a log
/// call site is enabled, and a test without a subscriber that reaches a call
/// site while the capturing test sets up its scoped subscriber can leave it
/// cached as disabled (see canary.rs).
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn host_count(core: &shellpp_core::Core) -> usize {
    core.records_query(RecordsQuery {
        vault_id: None,
        types: vec![RecordType::Host],
        scope: QueryScope::All,
        text: None,
        tags: Vec::new(),
        sort: SortMode::Manual,
    })
    .expect("query")
    .len()
}

/// Writes the attempt counter straight into `meta` while no core is open.
fn seed_lockout(env: &Env, value: &str) {
    let (mut store, _) = Store::open(&env.config().data_dir, common::APP_VERSION).expect("store");
    store
        .write(|tx| tx.meta_set(META_UNLOCK_LOCKOUT, value))
        .expect("meta");
}

#[test]
fn eight_characters_are_enough_and_there_are_no_composition_rules() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let short = core
        .first_run_setup_with(SecretString::from("abcdefg"), FirstRunOptions::default())
        .unwrap_err();
    assert_eq!(
        common::reasons(short),
        vec![("password".to_owned(), ValidationReason::TooShort)]
    );
    assert_eq!(core.lock_status().state, LockState::FirstRun);
    // Eight equal lowercase letters: accepted.
    let status = core
        .first_run_setup_with(
            SecretString::from("aaaaaaaa"),
            FirstRunOptions {
                remember_device: false,
            },
        )
        .expect("first run");
    assert_eq!(status.state, LockState::Unlocked);
    assert!(!status.remember_device_enabled);
    assert!(env.mem.secret_store.accounts().is_empty());
    // Auto-lock is "Never" by default.
    assert_eq!(
        core.settings_get().unwrap()["security.auto_lock_minutes"],
        serde_json::Value::Null
    );
}

#[test]
fn remember_on_this_device_is_on_by_default_at_first_run() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    assert!(FirstRunOptions::default().remember_device);
    let status = core
        .first_run_setup_with(new_password(), FirstRunOptions::default())
        .expect("first run");
    assert_eq!(status.state, LockState::Unlocked);
    assert!(status.remember_device_enabled);
    let accounts = env.mem.secret_store.accounts();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].starts_with("device-key:"));
    // The recovery key is kept in the vault but never offered for copying.
    assert!(matches!(
        core.secret_copy(shellpp_core::service::dto::SecretCopyTarget::RecoveryKey),
        Err(CoreError::WrongState)
    ));
    assert_eq!(
        names(&env.take_events()),
        vec!["lock:changed", "vaults:changed", "vaults:changed"]
    );

    // The next launch opens without the password (the in-memory keychain).
    drop(core);
    let core = env.open();
    assert_eq!(core.lock_status().state, LockState::Locked);
    assert!(core.try_device_unlock().expect("device unlock"));
    assert!(core.is_unlocked());
}

#[test]
fn without_an_os_keychain_the_first_run_still_succeeds() {
    let _serial = serial();
    let env = Env::new();
    env.mem.secret_store.set_available(false);
    let core = env.open();
    let status = core
        .first_run_setup_with(new_password(), FirstRunOptions::default())
        .expect("first run");
    assert_eq!(status.state, LockState::Unlocked);
    assert!(!status.remember_device_available);
    assert!(!status.remember_device_enabled);
}

#[test]
fn wrong_passwords_pause_unlock_and_the_pause_survives_a_restart() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let password = new_password();
    core.first_run_setup(password.clone()).expect("first run");
    core.lock().unwrap();
    assert_eq!(core.unlock_failures(), 0);
    assert_eq!(core.unlock_retry_after(), None);
    drop(core);

    // Four wrong passwords already (as after four attempts in an earlier run).
    seed_lockout(&env, r#"{"failures":4}"#);
    let core = env.open();
    assert_eq!(core.unlock_failures(), 4);
    assert_eq!(core.unlock_retry_after(), None);

    // The fifth wrong password starts a 5 s pause.
    assert!(matches!(
        core.unlock(new_password()),
        Err(CoreError::InvalidPassword)
    ));
    assert_eq!(core.unlock_failures(), 5);
    let left = core.unlock_retry_after().expect("paused");
    assert!(left <= Duration::from_secs(5) && left > Duration::ZERO);
    // During the pause even the right password is refused before Argon2id.
    match core.unlock(password.clone()) {
        Err(CoreError::TooManyAttempts { retry_after_secs }) => {
            assert!((1..=5).contains(&retry_after_secs), "{retry_after_secs}");
        }
        other => panic!("expected too_many_attempts, got {other:?}"),
    }
    let error = core.unlock(password.clone()).unwrap_err();
    assert_eq!(error.code(), "too_many_attempts");
    assert!(error.details()["retry_after_secs"].as_u64().is_some());
    assert_eq!(core.lock_status().state, LockState::Locked);

    // A restart keeps the pause.
    drop(core);
    let core = env.open();
    assert_eq!(core.unlock_failures(), 5);
    assert!(matches!(
        core.unlock(password.clone()),
        Err(CoreError::TooManyAttempts { .. })
    ));

    // After the pause the right password opens the vault and resets the counter.
    env.mem.clock.advance(Duration::from_secs(6));
    assert_eq!(core.unlock_retry_after(), None);
    core.unlock(password.clone())
        .expect("unlock after the pause");
    assert_eq!(core.unlock_failures(), 0);
    core.lock().unwrap();
    drop(core);
    let core = env.open();
    assert_eq!(core.unlock_failures(), 0);
    assert_eq!(core.unlock_retry_after(), None);
}

/// The limit holds for concurrent callers too (two windows, each with its
/// own unlock screen): the attempt that earns the pause counts from the
/// moment it starts, so a second attempt running at the same time is
/// refused before Argon2id instead of being a free guess.
#[test]
fn concurrent_unlocks_cannot_get_around_the_attempt_limit() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let password = new_password();
    core.first_run_setup(password.clone()).expect("first run");
    core.lock().unwrap();
    drop(core);
    seed_lockout(&env, r#"{"failures":4}"#);
    let core = Arc::new(env.open());

    let start = Arc::new(std::sync::Barrier::new(2));
    let attempts: Vec<_> = (0..2)
        .map(|_| {
            let core = core.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                core.unlock(new_password())
            })
        })
        .collect();
    let mut results: Vec<&'static str> = attempts
        .into_iter()
        .map(|attempt| match attempt.join().expect("thread") {
            Err(CoreError::InvalidPassword) => "invalid_password",
            Err(CoreError::TooManyAttempts { .. }) => "too_many_attempts",
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    results.sort_unstable();
    assert_eq!(results, vec!["invalid_password", "too_many_attempts"]);
    assert_eq!(core.unlock_failures(), 5);
    assert!(core.unlock_retry_after().is_some());

    // A right password after the pause resets the reserved attempt.
    env.mem.clock.advance(Duration::from_secs(6));
    core.unlock(password).expect("right password");
    assert_eq!(core.unlock_failures(), 0);
}

/// Turning "Remember on this device" off when the OS keychain refuses to
/// delete the item changes nothing (the option stays on, so Settings can
/// report it and try again); when it works, the item and the `device` row
/// go and the WAL is checkpointed, so the old wrap does not linger there.
#[test]
fn remember_off_keeps_the_pair_when_the_keychain_refuses() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    core.first_run_setup_with(new_password(), FirstRunOptions::default())
        .expect("first run");
    assert!(core.lock_status().remember_device_enabled);
    let accounts = env.mem.secret_store.accounts();
    assert_eq!(accounts.len(), 1);

    env.mem.secret_store.set_available(false);
    assert!(matches!(
        core.remember_device_set(false),
        Err(CoreError::Unavailable)
    ));
    env.mem.secret_store.set_available(true);
    assert!(core.lock_status().remember_device_enabled, "still on");
    assert_eq!(env.mem.secret_store.accounts(), accounts, "item kept");

    let status = core.remember_device_set(false).expect("remember off");
    assert!(!status.remember_device_enabled);
    assert!(env.mem.secret_store.accounts().is_empty());
    let wal = env.config().data_dir.join("shellpp.db-wal");
    let wal_len = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert_eq!(wal_len, 0, "the WAL was checkpointed and truncated");
}

#[test]
fn the_next_wrong_password_doubles_the_pause_up_to_five_minutes() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let password = new_password();
    core.first_run_setup(password).expect("first run");
    core.lock().unwrap();
    drop(core);
    // Ten wrong passwords and an expired pause: the eleventh earns 300 s.
    seed_lockout(&env, r#"{"failures":10,"retry_at_ms":0}"#);
    let core = env.open();
    assert_eq!(core.unlock_retry_after(), None);
    assert!(matches!(
        core.unlock(new_password()),
        Err(CoreError::InvalidPassword)
    ));
    let left = core.unlock_retry_after().expect("paused");
    assert!(left > Duration::from_secs(290) && left <= Duration::from_secs(300));
    // A clock set back does not lengthen it.
    env.mem.clock.rewind(Duration::from_secs(3600));
    assert!(core.unlock_retry_after().unwrap() <= Duration::from_secs(300));
}

#[test]
fn start_with_a_new_vault_renames_the_data_file_and_returns_to_first_run() {
    let _serial = serial();
    let env = Env::new();
    let core = env.open();
    let password = new_password();
    core.first_run_setup(password.clone()).expect("first run");
    let vault_id = core.vaults().unwrap()[0].vault_id;
    common::save(
        &core,
        vault_id,
        None,
        RecordType::Host,
        json!({"address": "db1.example.com"}),
        &[],
    )
    .expect("host");
    // Only while locked.
    assert!(matches!(core.start_new_vault(), Err(CoreError::WrongState)));
    core.lock().unwrap();
    env.take_events();

    let archived = core.start_new_vault().expect("new vault");
    assert!(
        archived.starts_with("shellpp.db.") && archived.ends_with(".old"),
        "{archived}"
    );
    assert!(!archived.contains('/') && !archived.contains('\\'));
    let data_dir = env.config().data_dir;
    assert!(data_dir.join(&archived).is_file(), "the old file is kept");
    assert!(data_dir.join("shellpp.db").is_file(), "a new database");
    assert_eq!(core.lock_status().state, LockState::FirstRun);
    let events = names(&env.take_events());
    assert_eq!(events.first(), Some(&"lock:changed"));
    assert!(events.contains(&"settings:changed"));
    // Not again at first run.
    assert!(matches!(core.start_new_vault(), Err(CoreError::WrongState)));

    // A new first run with a new password: an empty vault.
    core.first_run_setup(new_password()).expect("new first run");
    assert_eq!(host_count(&core), 0);
    drop(core);

    // The archive is the old database: the old password still opens it.
    let copy = tempfile::tempdir().unwrap();
    std::fs::copy(data_dir.join(&archived), copy.path().join("shellpp.db")).unwrap();
    let old = shellpp_core::Core::open(
        shellpp_core::CoreConfig {
            data_dir: copy.path().to_path_buf(),
            app_version: common::APP_VERSION.to_owned(),
        },
        env.mem
            .platform(shellpp_core::platform::OsKind::Windows, "TEST-PC"),
    )
    .expect("archive opens");
    old.unlock(password).expect("old password");
    assert_eq!(host_count(&old), 1);
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
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

/// The master password never reaches the log or `Debug` output on the new
/// paths: first run with "remember", wrong passwords and the pause, and the
/// new-vault reset.
#[test]
fn the_master_password_never_reaches_logs_or_debug_output() {
    let _serial = serial();
    let canary = format!("Canary-{}-pw", common::random_hex());
    let wrong = format!("Wrong-{}-pw", common::random_hex());
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let mut debug = String::new();
    tracing::subscriber::with_default(subscriber, || {
        let env = Env::new();
        let core = env.open();
        let status = core
            .first_run_setup_with(
                SecretString::from(canary.clone()),
                FirstRunOptions::default(),
            )
            .expect("first run");
        debug.push_str(&format!("{status:?} {core:?}"));
        core.lock().unwrap();
        drop(core);
        seed_lockout(&env, r#"{"failures":4}"#);
        let core = env.open();
        let error = core.unlock(SecretString::from(wrong.clone())).unwrap_err();
        debug.push_str(&format!("{error:?} {}", error.details()));
        let error = core.unlock(SecretString::from(canary.clone())).unwrap_err();
        debug.push_str(&format!("{error:?} {}", error.details()));
        let archived = core.start_new_vault().expect("new vault");
        debug.push_str(&archived);
    });
    let logs = capture.0.lock().unwrap().clone();
    let logs = String::from_utf8_lossy(&logs);
    assert!(
        logs.contains("wrong master password; unlock paused"),
        "the subscriber captured the core's logs"
    );
    for secret in [&canary, &wrong] {
        assert!(!logs.contains(secret.as_str()), "a password in the log");
        assert!(
            !debug.contains(secret.as_str()),
            "a password in Debug output"
        );
    }
}
