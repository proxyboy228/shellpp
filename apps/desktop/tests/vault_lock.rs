//! Owner's stage 1, step 5.1 "encryption: round-trip and a wrong password",
//! at the app level (the app state and the unlock screen over a real core in
//! a temporary profile; in-memory secret store, headless GPUI):
//!
//! - what the app saves is not readable in the files of the profile, and a
//!   restart with the master password brings it back (round-trip);
//! - a wrong master password is refused, keeps everything locked and is
//!   counted; after 5 wrong passwords in a row the core pauses unlocking
//!   (decision D29), refuses even the right password during the pause, and
//!   keeps the counter across a restart.

mod common;

use std::time::Duration;

use common::{Ui, map, visible};
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use shellpp_core::CoreError;
use shellpp_core::service::dto::LockState;
use shellpp_desktop::app::root::Screen;
use shellpp_desktop::state::records_edit::{HostDraft, HostSecrets, SecretChange};
use shellpp_desktop::testing::TestProfile;
use uuid::Uuid;

fn random_text(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

/// Every file under `root`, with its bytes.
fn files_under(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                files.push((path, bytes));
            }
        }
    }
    files
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Unlocks through the app state (Argon2id on the background executor).
async fn unlock(ui: &Ui, cx: &mut TestAppContext, password: &str) -> Result<(), CoreError> {
    let task = ui.state.update(cx, |state, cx| {
        state.unlock(SecretString::from(password.to_owned()), cx)
    });
    let result = task.await;
    cx.run_until_parked();
    result
}

/// Round-trip: the label, notes, group name and the host password are not
/// in any file of the profile (database, WAL, logs) in plain text; after a
/// restart the profile opens locked with nothing readable, and the master
/// password brings everything back.
#[gpui_kit::test]
async fn saved_data_is_encrypted_and_reopens_with_the_password(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let master = profile.password().expose_secret().to_owned();
    let label = random_text("Label");
    let notes = random_text("Notes");
    let group_name = random_text("Group");
    let secret = random_text("Secret");

    let ui = Ui::launch(cx, &mut profile);
    let (group, host) = ui.state.update(cx, |state, cx| {
        let group = state
            .save_group(None, map(json!({"name": group_name})), cx)
            .expect("group")
            .record_id;
        let draft = HostDraft {
            address: "db.example.com".into(),
            label: label.clone(),
            notes: notes.clone(),
            group_id: Some(group),
            ..Default::default()
        };
        let secrets = HostSecrets {
            password: SecretChange::Set(SecretString::from(secret.clone())),
            ..Default::default()
        };
        let report = state.save_host_draft(None, &draft, &secrets, None, cx);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        (group, report.saved.expect("host").record_id)
    });
    ui.close(cx);

    // At rest: nothing of it in plain text, in any file of the profile.
    let files = files_under(profile.path());
    assert!(
        files.iter().any(|(path, _)| path
            .file_name()
            .is_some_and(|name| name == shellpp_core::ids::DB_FILE_NAME)),
        "the database file exists"
    );
    for (path, bytes) in &files {
        for plain in [&label, &notes, &group_name, &secret, &master] {
            assert!(
                !contains(bytes, plain.as_bytes()),
                "plain text found in {}",
                path.display()
            );
        }
    }

    // Restart: locked, nothing readable without the password.
    let ui = Ui::launch_locked(cx, &mut profile);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    cx.update(|cx| {
        let state = ui.state.read(cx);
        assert_eq!(state.lock_state(), LockState::Locked);
        assert!(state.hosts().is_empty() && state.groups().is_empty());
    });

    // A wrong password does not open it.
    let wrong = format!("{master}x");
    assert!(matches!(
        unlock(&ui, cx, &wrong).await,
        Err(CoreError::InvalidPassword)
    ));
    cx.update(|cx| {
        let state = ui.state.read(cx);
        assert_eq!(state.lock_state(), LockState::Locked);
        assert!(state.hosts().is_empty());
    });

    // The right one brings everything back.
    unlock(&ui, cx, &master)
        .await
        .expect("unlock with the master password");
    assert_eq!(ui.screen(cx), Screen::Shell);
    let item = ui.host(cx, host).expect("the host");
    assert_eq!(item.label.as_deref(), Some(label.as_str()));
    assert_eq!(item.data["notes"], notes.as_str());
    assert_eq!(item.group_id, Some(group));
    let revealed = cx
        .update(|cx| ui.state.read(cx).reveal_secret(host, "ssh.password"))
        .expect("the password");
    assert_eq!(revealed.expose_secret(), secret);
    ui.close(cx);
}

/// Types `password` into the unlock screen and presses Enter.
fn submit_on_unlock_screen(ui: &Ui, cx: &mut TestAppContext, password: &str) {
    ui.frame(cx, |window, cx| {
        window.click("unlock-password", cx);
        window.input(password, cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
}

/// A wrong password on the unlock screen shows the error, keeps the lock and
/// is counted; 5 in a row start a pause the core enforces (the right
/// password is refused too, before any key derivation); the counter and the
/// pause survive a restart; after the pause the right password unlocks and
/// resets the counter.
#[gpui_kit::test]
async fn wrong_passwords_are_refused_counted_and_paused(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let master = profile.password().expose_secret().to_owned();
    // First run, then locked.
    let ui = Ui::launch(cx, &mut profile);
    ui.state
        .update(cx, |state, cx| state.lock(cx))
        .expect("lock");
    ui.close(cx);

    let ui = Ui::launch_locked(cx, &mut profile);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    for attempt in 1..=4u32 {
        submit_on_unlock_screen(&ui, cx, &format!("wrong-{attempt}-{master}"));
        ui.frame(cx, |window, _| {
            assert!(visible(window, "unlock-error"), "attempt {attempt}");
        });
        assert_eq!(ui.screen(cx), Screen::Unlock);
        assert_eq!(ui.core(cx).unlock_failures(), attempt);
        assert_eq!(
            ui.core(cx).unlock_retry_after(),
            None,
            "no pause before the fifth"
        );
    }
    // The fifth wrong password starts a 5 s pause.
    submit_on_unlock_screen(&ui, cx, &format!("wrong-5-{master}"));
    assert_eq!(ui.core(cx).unlock_failures(), 5);
    let pause = ui.core(cx).unlock_retry_after().expect("a pause");
    assert!(pause <= Duration::from_secs(5) && pause > Duration::from_secs(3));
    // Enforced in the core: even the right password is refused now.
    assert!(matches!(
        unlock(&ui, cx, &master).await,
        Err(CoreError::TooManyAttempts { retry_after_secs }) if (1..=5).contains(&retry_after_secs)
    ));
    assert_eq!(ui.screen(cx), Screen::Unlock);
    assert_eq!(
        ui.core(cx).unlock_failures(),
        5,
        "a refused attempt is not counted"
    );
    ui.close(cx);

    // Restart: the counter and the pause are kept.
    let ui = Ui::launch_locked(cx, &mut profile);
    assert_eq!(ui.core(cx).unlock_failures(), 5);
    assert!(ui.core(cx).unlock_retry_after().is_some(), "still paused");
    assert!(matches!(
        unlock(&ui, cx, &master).await,
        Err(CoreError::TooManyAttempts { .. })
    ));
    // After the pause (the profile's clock moves on), the right password
    // unlocks and the counter starts over.
    profile
        .memory()
        .expect("in-memory platform")
        .clock
        .advance(Duration::from_secs(6));
    assert_eq!(ui.core(cx).unlock_retry_after(), None);
    unlock(&ui, cx, &master)
        .await
        .expect("unlock after the pause");
    assert_eq!(ui.screen(cx), Screen::Shell);
    assert_eq!(ui.core(cx).unlock_failures(), 0);
    ui.close(cx);

    let ui = Ui::launch_locked(cx, &mut profile);
    assert_eq!(ui.core(cx).unlock_failures(), 0, "the reset is kept too");
    ui.close(cx);
}
