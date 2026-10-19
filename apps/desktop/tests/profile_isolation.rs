//! Owner's step 0: tests never touch the permanent profile.
//!
//! These tests run the real `shellpp` binary with `--print-data-dir`, which
//! resolves the data directory exactly like a normal launch and exits before
//! creating anything or opening a window. The base-directory variables are
//! pointed at an empty temporary directory, which must stay empty.

use std::path::Path;
use std::process::{Command, Output};

use shellpp_core::ids;
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
