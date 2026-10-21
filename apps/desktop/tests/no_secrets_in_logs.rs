//! Owner's stage 1, step 5.1: the master password never appears in the logs
//! (docs/security.md, section 9.1).
//!
//! The same app-level flow runs twice, each time over its own temporary
//! profile with the in-memory secret store (headless GPUI):
//!
//! 1. first run on the first-run screen (the password typed twice), lock,
//!    a wrong password and the right one on the unlock screen, a password
//!    change, lock, unlock with the new password;
//! 2. it is checked for every password used (the master password, the new
//!    one, the wrong one, all with non-ASCII characters):
//!    - [`log_file_and_stderr_have_no_password`]: the real logging of the app
//!      (`logging::init`: the log file in the profile, plus stderr in debug
//!      builds) in a child process of this test binary, so its stderr can be
//!      captured; every log file and the child's whole output are searched;
//!    - [`trace_output_has_no_password`]: every tracing event and span of
//!      every crate at TRACE level (more than the app writes), captured in
//!      memory in this process.
//!
//! "Absent" means none of: the UTF-8 bytes, the random part of the password
//! (catches a partial leak), the Debug form of its bytes, hex and the
//! Unicode-escaped form.
//!
//! Stage 3 adds the Keychain ([`keychain_flows_leave_no_key_or_passphrase_in_the_trace`]):
//! key passphrases and private key text (every line of a key's body) never
//! reach the TRACE output either.

mod common;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use common::setup;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use secrecy::SecretString;
use shellpp_core::service::dto::LockState;
use shellpp_core::{Core, CoreConfig};
use shellpp_desktop::app::{self, RootView, root::Screen};
use shellpp_desktop::dirs::AppDirs;
use shellpp_desktop::platform::{self, events::event_channel};
use shellpp_desktop::profile::{TempProfile, random_suffix};
use shellpp_desktop::state::{AppState, OpenedCore};

/// Set for the child process: the profile folder of its flow.
const ENV_CHILD_DIR: &str = "SHELLPP_LOG_TEST_CHILD_DIR";
/// The test the child process runs.
const CHILD_TEST: &str = "child_runs_the_flow_with_the_app_logging";

/// The passwords of one flow; each has a random ASCII part.
#[derive(Clone, Debug)]
struct Passwords {
    master: String,
    new: String,
    wrong: String,
}

impl Passwords {
    fn generate() -> Self {
        let random = || random_suffix().expect("OS random");
        Self {
            master: format!("Мастер-{}-пароль✓", random()),
            new: format!("Новый-{}-ключ🔑", random()),
            wrong: format!("Неверный-{}-ü", random()),
        }
    }

    fn all(&self) -> [&str; 3] {
        [&self.master, &self.new, &self.wrong]
    }

    /// One line each, for the child's stdin.
    fn to_lines(&self) -> String {
        format!("{}\n{}\n{}\n", self.master, self.new, self.wrong)
    }

    fn from_lines(text: &str) -> Self {
        let mut lines = text.lines();
        let mut next = || lines.next().expect("three passwords").to_owned();
        Self {
            master: next(),
            new: next(),
            wrong: next(),
        }
    }
}

/// The forms of `password` that must not appear in `haystack`.
fn leaks(haystack: &[u8], password: &str) -> Vec<&'static str> {
    let contains = |needle: &[u8]| {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|window| window == needle)
    };
    let random_part = password
        .split('-')
        .find(|part| part.len() >= 16 && part.chars().all(|c| c.is_ascii_hexdigit()))
        .expect("a random part");
    let bytes = password.as_bytes();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let checks: [(&'static str, Vec<u8>); 6] = [
        ("UTF-8 bytes", bytes.to_vec()),
        ("random part", random_part.as_bytes().to_vec()),
        ("Debug of the bytes", format!("{bytes:?}").into_bytes()),
        (
            "Debug of the bytes without spaces",
            format!("{bytes:?}").replace(' ', "").into_bytes(),
        ),
        ("hex", hex.into_bytes()),
        (
            "Unicode-escaped",
            password.escape_unicode().to_string().into_bytes(),
        ),
    ];
    checks
        .into_iter()
        .filter(|(_, needle)| contains(needle))
        .map(|(name, _)| name)
        .collect()
}

/// Fails if any password leaks into `text` (named `what`). The message
/// never repeats the password.
fn assert_no_password(what: &str, text: &[u8], passwords: &Passwords) {
    for (name, password) in [
        ("master password", &passwords.master),
        ("new password", &passwords.new),
        ("wrong password", &passwords.wrong),
    ] {
        let found = leaks(text, password);
        assert!(found.is_empty(), "{what}: the {name} leaked ({found:?})");
    }
}

fn window(
    state: &Entity<AppState>,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<RootView>) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1280.), px(800.)),
        ))),
        ..Default::default()
    };
    cx.update(|cx| app::open_window_with(state.clone(), options, cx))
        .expect("test window")
}

fn screen(root: &Entity<RootView>, cx: &mut TestAppContext) -> Screen {
    cx.update(|cx| root.read(cx).screen(cx))
}

fn lock_state(state: &Entity<AppState>, cx: &mut TestAppContext) -> LockState {
    cx.update(|cx| state.read(cx).lock_state())
}

/// Types into the input `id` and presses Enter.
fn type_and_enter(handle: AnyWindowHandle, id: &'static str, text: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        window.input(text, cx);
        window.press("enter", cx);
    })
    .expect("window");
    cx.run_until_parked();
}

/// The first-run screen: the password twice, then "Create vault"
/// ("Remember on this device" stays on, its default: the device key goes to
/// the in-memory secret store).
fn first_run_on_screen(
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    password: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("first-run-password", cx);
        window.input(password, cx);
        window.click("first-run-repeat", cx);
        window.input(password, cx);
        window.render_frame(cx);
        let screen = root.read(cx).first_run_screen().expect("first-run screen");
        let screen = screen.read(cx);
        assert!(
            screen.can_create(cx),
            "Create vault is enabled (password error {:?}, repeat error {:?})",
            screen.password_error(cx),
            screen.repeat_error(cx)
        );
        window.click("first-run-create", cx);
    })
    .expect("window");
    cx.run_until_parked();
}

/// The whole flow in `dir` (a fresh profile folder). Returns when the app
/// state has shut down and the log file is closed.
fn run_flow(dir: &Path, passwords: &Passwords, cx: &mut TestAppContext) {
    setup(cx);
    let dirs = AppDirs::under(dir.to_path_buf());
    dirs.create().expect("profile folders");
    let (sink, events) = event_channel();
    let (platform, _memory) = platform::in_memory(sink);
    let core = Core::open(
        CoreConfig {
            data_dir: dirs.data.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        platform,
    )
    .expect("core");
    let core = Arc::new(core);
    let opened = OpenedCore {
        core: core.clone(),
        events,
        extras: Default::default(),
        dirs: Some(dirs),
        temp_profile: None,
    };
    let state = cx.new(|cx| AppState::new(opened, cx));
    let (handle, root) = window(&state, cx);
    assert_eq!(screen(&root, cx), Screen::FirstRun);

    // First run on the first-run screen.
    first_run_on_screen(handle, &root, &passwords.master, cx);
    assert_eq!(lock_state(&state, cx), LockState::Unlocked, "first run");

    // Lock, a wrong password, the right one.
    state.update(cx, |state, cx| state.lock(cx)).expect("lock");
    cx.run_until_parked();
    assert_eq!(screen(&root, cx), Screen::Unlock);
    type_and_enter(handle, "unlock-password", &passwords.wrong, cx);
    assert_eq!(lock_state(&state, cx), LockState::Locked, "wrong password");
    type_and_enter(handle, "unlock-password", &passwords.master, cx);
    assert_eq!(
        lock_state(&state, cx),
        LockState::Unlocked,
        "right password"
    );

    // A password change, then lock and unlock with the new password.
    core.password_change(
        SecretString::from(passwords.master.clone()),
        SecretString::from(passwords.new.clone()),
    )
    .expect("password change");
    state.update(cx, |state, cx| state.lock(cx)).expect("lock");
    cx.run_until_parked();
    type_and_enter(handle, "unlock-password", &passwords.new, cx);
    assert_eq!(lock_state(&state, cx), LockState::Unlocked, "new password");

    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    cx.run_until_parked();
    drop(core);
    state.update(cx, |state, _| state.shutdown());
    shellpp_desktop::logging::close_file();
}

/// The child half of [`log_file_and_stderr_have_no_password`]: runs only in
/// the child process (with [`ENV_CHILD_DIR`]); a no-op in a normal test run.
#[gpui_kit::test]
fn child_runs_the_flow_with_the_app_logging(cx: &mut TestAppContext) {
    let Some(dir) = std::env::var_os(ENV_CHILD_DIR) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).expect("stdin");
    let passwords = Passwords::from_lines(&input);
    // The app's own logging, as `main` sets it up: the folders first, then
    // the rotating log file in the logs folder and, in debug builds, stderr.
    let dirs = AppDirs::under(dir.clone());
    dirs.create().expect("profile folders");
    shellpp_desktop::logging::init(&dirs.logs);
    run_flow(&dir, &passwords, cx);
}

/// The log file and the stderr output of the app's real logging hold no
/// password.
#[test]
fn log_file_and_stderr_have_no_password() {
    let profile = TempProfile::create("shellpp-test-logs").expect("temporary folder");
    let passwords = Passwords::generate();
    let mut child = Command::new(std::env::current_exe().expect("test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(ENV_CHILD_DIR, profile.path())
        .env(shellpp_core::ids::ENV_DATA_DIR, profile.path())
        .env(shellpp_desktop::ENV_TEST, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("child test process");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(passwords.to_lines().as_bytes())
        .expect("passwords to the child");
    let output = child.wait_with_output().expect("child output");

    // Check before showing anything of the output.
    assert_no_password("the child's stderr", &output.stderr, &passwords);
    assert_no_password("the child's stdout", &output.stdout, &passwords);
    // (Shown on failure: both have passed the leak check above.)
    assert!(
        output.status.success(),
        "the child flow failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("{CHILD_TEST} ... ok")) || stdout.contains("1 passed"),
        "the child ran the flow:\n{stdout}"
    );

    // Every log file (shellpp.log and rotated ones).
    let logs = AppDirs::under(profile.path().to_path_buf()).logs;
    let mut log_bytes = Vec::new();
    for entry in std::fs::read_dir(&logs).expect("logs folder").flatten() {
        let bytes = std::fs::read(entry.path()).expect("log file");
        assert_no_password(
            &format!("log file {:?}", entry.file_name()),
            &bytes,
            &passwords,
        );
        log_bytes.extend(bytes);
    }
    // The capture works: the app's events of the flow are there.
    // (Shown on failure: it has passed the leak check above.)
    let log_text = String::from_utf8_lossy(&log_bytes);
    assert!(
        log_text.matches("shellpp_desktop").count() >= 3,
        "the log file has the app's events:\n{log_text}"
    );
    if cfg!(debug_assertions) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.matches("shellpp_desktop").count() >= 3,
            "debug builds also log to stderr:\n{stderr}"
        );
    }
}

/// An in-memory writer for the tracing capture.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Every tracing event and span (all crates, TRACE) during the flow holds
/// no password.
#[gpui_kit::test]
fn trace_output_has_no_password(cx: &mut TestAppContext) {
    let profile = TempProfile::create("shellpp-test-logs").expect("temporary folder");
    let passwords = Passwords::generate();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_target(true)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_writer(captured.clone())
        .finish();
    {
        // This thread runs the whole flow, including the background
        // executor of the GPUI test platform (Argon2id).
        let _guard = tracing::subscriber::set_default(subscriber);
        run_flow(profile.path(), &passwords, cx);
    }
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_no_password("the TRACE output", &bytes, &passwords);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.matches("shellpp_desktop").count() >= 3 && text.contains("shellpp_core"),
        "the capture saw the flow"
    );
}

/// The lines of the base64 body of an OpenSSH private key (its armor lines
/// are not secret).
fn key_body_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("-----"))
        .map(str::to_owned)
        .collect()
}

/// Fails if a line of a private key's body leaks into `text`.
fn assert_no_key(what: &str, text: &[u8], key: &str) {
    let lines = key_body_lines(key);
    assert!(!lines.is_empty(), "{what}: a key body to look for");
    for (index, line) in lines.iter().enumerate() {
        let leaked = text
            .windows(line.len())
            .any(|window| window == line.as_bytes());
        assert!(!leaked, "{what}: line {index} of a private key leaked");
    }
}

/// The Keychain (UI 7.3, docs/security.md 9.1): a key generated with a
/// passphrase on the Generate key form, an encrypted key pasted into New key
/// with its passphrase, Show and Copy private key. Every tracing event and
/// span (all crates, TRACE) holds neither passphrase nor any line of either
/// private key.
#[gpui_kit::test]
fn keychain_flows_leave_no_key_or_passphrase_in_the_trace(cx: &mut TestAppContext) {
    use shellpp_core::keys::generate::{KeyKind, generate};
    use shellpp_desktop::keychain::{self, KeychainScreen};
    use shellpp_desktop::testing::TestProfile;

    let random = || random_suffix().expect("OS random");
    let generate_passphrase = format!("Gen-{}-pass", random());
    let import_passphrase = format!("Imp-{}-pass", random());
    let imported_text = generate(
        KeyKind::Ed25519,
        "imported@test",
        Some(&SecretString::from(import_passphrase.clone())),
    )
    .expect("key")
    .to_string();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_target(true)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_writer(captured.clone())
        .finish();
    let generated_text;
    {
        // This thread runs the whole flow, the background executor of the
        // GPUI test platform included (generation, bcrypt-pbkdf).
        let _guard = tracing::subscriber::set_default(subscriber);
        setup(cx);
        cx.update(|cx| keychain::set_ssh_folder(None, cx));
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let memory = profile.memory().expect("memory platform").clone();
        let state = cx.new(|cx| AppState::new(opened, cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(1000.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| KeychainScreen::new(screen_state, window, cx))
                })
            })
            .expect("window");
        let act = |cx: &mut TestAppContext,
                   f: &mut dyn FnMut(&mut gpui_kit::Window, &mut gpui_kit::App)| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx);
            })
            .expect("window");
            cx.run_until_parked();
        };
        let type_into = |cx: &mut TestAppContext, id: &'static str, text: &str| {
            act(cx, &mut |window, cx| {
                window.click(id, cx);
                window.input(text, cx);
            });
        };

        // Generate key with a passphrase.
        act(cx, &mut |window, cx| {
            screen.update(cx, |screen, cx| screen.open_generate(window, cx))
        });
        type_into(cx, "kc-gen-label", "Generated");
        type_into(cx, "kc-gen-passphrase", &generate_passphrase);
        // Typed twice (D64).
        type_into(cx, "kc-gen-passphrase-repeat", &generate_passphrase);
        act(cx, &mut |window, cx| window.click("kc-gen-submit", cx));
        let generated = cx
            .update(|cx| screen.read(cx).selected())
            .expect("the generated key is shown");

        // New key: an encrypted key with its passphrase.
        act(cx, &mut |window, cx| {
            screen.update(cx, |screen, cx| screen.open_import(window, cx));
            let form = screen.read(cx).import_form().cloned().expect("import form");
            form.update(cx, |form, cx| form.set_text(&imported_text, window, cx));
        });
        type_into(cx, "kc-import-passphrase", &import_passphrase);
        type_into(cx, "kc-import-label", "Imported");
        act(cx, &mut |window, cx| window.click("kc-import-submit", cx));
        let imported = cx
            .update(|cx| screen.read(cx).selected())
            .expect("the imported key is shown");
        assert_ne!(imported, generated);

        // Show and Copy private key, for both keys.
        let mut copied = Vec::new();
        for key in [generated, imported] {
            act(cx, &mut |window, cx| {
                window.click(keychain::key_card_id(key), cx);
            });
            act(cx, &mut |window, cx| window.click("kc-key-show", cx));
            act(cx, &mut |window, cx| {
                window.click("kc-key-copy-private", cx)
            });
            copied.push(memory.clipboard.text().expect("copied"));
        }
        assert_eq!(copied[1], imported_text.trim());
        generated_text = copied[0].clone();

        let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        state.update(cx, |state, _| state.shutdown());
        drop(profile);
    }
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    for (name, passphrase) in [
        ("the Generate key passphrase", &generate_passphrase),
        ("the import passphrase", &import_passphrase),
    ] {
        let found = leaks(&bytes, passphrase);
        assert!(
            found.is_empty(),
            "the TRACE output: {name} leaked ({found:?})"
        );
    }
    assert_no_key("the TRACE output", &bytes, &generated_text);
    assert_no_key("the TRACE output", &bytes, &imported_text);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("key generated") && text.contains("key imported"),
        "the capture saw the flow"
    );
}

#[test]
fn the_leak_check_finds_every_form() {
    let passwords = Passwords::generate();
    let password = passwords.master.as_str();
    let random = password.split('-').nth(1).unwrap();
    for (index, haystack) in [
        format!("x {password} y"),
        format!("password=\"{random}\""),
        format!("{:?}", password.as_bytes()),
        password
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        password.escape_unicode().to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !leaks(haystack.as_bytes(), password).is_empty(),
            "form {index}"
        );
    }
    assert!(leaks(b"nothing here", password).is_empty());
    assert_eq!(
        Passwords::from_lines(&passwords.to_lines()).all(),
        passwords.all()
    );
}
