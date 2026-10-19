//! Owner's step 0: tests never touch the permanent profile.
//!
//! These tests run the real `shellpp` binary with `--print-data-dir`, which
//! resolves the data directory exactly like a normal launch and exits before
//! creating anything or opening a window. The base-directory variables are
//! pointed at an empty temporary directory, which must stay empty.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;
use shellpp_core::ids;
use shellpp_core::service::dto::RecordView;
use shellpp_desktop::hosts::host_card_id;
use shellpp_desktop::profile::TempProfile;

const BINARY: &str = env!("CARGO_BIN_EXE_shellpp");

/// The binary with a scrubbed environment: no `SHELLPP_DATA_DIR`, and every
/// variable the OS directory lookup may read pointed into `fake_home`.
fn shellpp(fake_home: &Path) -> Command {
    let mut command = Command::new(BINARY);
    command
        .env_remove(ids::ENV_DATA_DIR)
        .env_remove(shellpp_desktop::ENV_TEST)
        .env("HOME", fake_home)
        .env("USERPROFILE", fake_home)
        .env("LOCALAPPDATA", fake_home.join("local"))
        .env("APPDATA", fake_home.join("roaming"))
        .env("XDG_DATA_HOME", fake_home.join("data"))
        .env("XDG_STATE_HOME", fake_home.join("state"))
        .env("XDG_CACHE_HOME", fake_home.join("cache"));
    command
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the shellpp binary runs")
}

fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

/// `SHELLPP_TEST=1` without `SHELLPP_DATA_DIR`: the launch fails loudly
/// before anything is created, so a test can never fall back to the
/// permanent profile.
#[test]
fn test_runs_without_a_data_dir_fail_loudly() {
    let fake_home = TempProfile::create("shellpp-test-home").unwrap();
    let output = run(shellpp(fake_home.path())
        .env(shellpp_desktop::ENV_TEST, "1")
        .arg("--print-data-dir"));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SHELLPP_DATA_DIR"), "{stderr}");
    assert!(output.stdout.is_empty());
    assert!(is_empty_dir(fake_home.path()), "nothing may be created");
}

/// With `SHELLPP_DATA_DIR`, a test run uses exactly that directory.
#[test]
fn test_runs_use_the_data_dir_they_are_given() {
    let fake_home = TempProfile::create("shellpp-test-home").unwrap();
    let data = TempProfile::create("shellpp-test-data").unwrap();
    let target = data.path().join("profile");
    let output = run(shellpp(fake_home.path())
        .env(shellpp_desktop::ENV_TEST, "1")
        .env(ids::ENV_DATA_DIR, &target)
        .arg("--print-data-dir"));
    assert!(output.status.success(), "{output:?}");
    let printed = String::from_utf8_lossy(&output.stdout);
    assert_eq!(Path::new(printed.trim()), target);
    // Resolution creates nothing, here or in the fake home.
    assert!(!target.exists());
    assert!(is_empty_dir(fake_home.path()));
}

/// The test helper's profiles live under the OS temporary directory, never
/// in the permanent profile, and are removed afterwards.
#[test]
fn test_profiles_are_temporary() {
    let mut profile = shellpp_desktop::testing::TestProfile::new();
    let path = profile.path().to_path_buf();
    assert!(path.starts_with(std::env::temp_dir()));
    if let Ok(permanent) = shellpp_desktop::dirs::permanent(
        shellpp_desktop::dirs::OsFamily::current(),
        &shellpp_desktop::dirs::BaseDirs::from_os(),
    ) {
        assert!(!path.starts_with(&permanent.data));
        assert!(!permanent.data.starts_with(&path));
    }
    let opened = profile.open_unlocked();
    assert!(path.join(ids::DB_FILE_NAME).is_file());
    drop(opened);
    drop(profile);
    assert!(!path.exists());
}

/// The permanent profile without `logs/` (a Shell++ instance the owner runs
/// at the same time may write its log; the tests never initialize logging).
fn permanent_snapshot(permanent: Option<&Path>) -> Option<Option<Vec<common::TreeEntry>>> {
    permanent.map(|path| {
        common::tree_snapshot(path).map(|entries| {
            entries
                .into_iter()
                .filter(|(relative, _, _)| !relative.starts_with("logs"))
                .collect()
        })
    })
}

/// Owner's steps 0 and 5: a whole UI flow in this test process (records
/// written through the app state and through the host panel's autosave,
/// moves, duplicates, removal, lock) runs on a temporary profile and leaves
/// the permanent profile (`%LOCALAPPDATA%\shellpp` on Windows) exactly as it
/// was: same entries, sizes and modification times, or still absent.
#[gpui_kit::test]
fn ui_flows_leave_the_permanent_profile_untouched(cx: &mut TestAppContext) {
    let permanent = common::permanent_profile();
    let before = permanent_snapshot(permanent.as_deref());

    let (ui, ids) = common::Ui::open(
        cx,
        &[
            json!({"address": "web1.example.com", "label": "Web"}),
            json!({"name": "Group"}),
        ],
    );
    let [web, group] = ids[..] else {
        unreachable!()
    };
    common::assert_temporary(&ui.profile);
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        window.click("host-label", cx);
        window.input(" edited", cx);
    });
    ui.wait(cx, common::AFTER_DEBOUNCE);
    assert_eq!(
        ui.host(cx, web).unwrap().label.as_deref(),
        Some("Web edited")
    );
    ui.state.update(cx, |state, cx| {
        state.move_records(vec![web], Some(group), cx).unwrap();
        let copy = match state.duplicate_record(web, cx).unwrap() {
            RecordView::Host(view) => view.record_id,
            RecordView::Group(view) => view.record_id,
        };
        state.delete_records(vec![copy], cx).unwrap();
        state.lock(cx).unwrap();
    });
    ui.settle(cx);
    ui.finish(cx);

    assert_eq!(
        permanent_snapshot(permanent.as_deref()),
        before,
        "the permanent profile changed during a test"
    );
}
