//! Headless UI tests of the lock screens (UI 13.1-13.3, decision D29; GPUI
//! `TestAppContext`, no real window): the first run creates the vault and
//! opens the empty Hosts screen (no recovery key in local mode, "Remember on
//! this device" on by default, 8 characters are enough); the unlock screen
//! rejects a wrong password and opens with the right one; locking keeps the
//! open session tabs but hides them until unlock. Every test uses its own
//! temporary profile (`TestProfile`) and the in-memory secret store.
//! The Hosts screen and the host panel are tested in `hosts_screen.rs` and
//! `host_panel.rs` (shared helpers in `common/mod.rs`).

mod common;

use common::{Ui, setup, visible};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use secrecy::ExposeSecret as _;
use shellpp_core::service::dto::LockState;
use shellpp_desktop::app::root::Screen;
use shellpp_desktop::app::{self, RootView};
use shellpp_desktop::shell::TopTab;
use shellpp_desktop::state::AppState;
use shellpp_desktop::testing::TestProfile;

fn open(state: &Entity<AppState>, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<RootView>) {
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

/// Types the password into both first-run fields and presses "Create vault".
fn create_vault(handle: AnyWindowHandle, password: &str, repeat: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("first-run-password", cx);
        window.input(password, cx);
        window.click("first-run-repeat", cx);
        window.input(repeat, cx);
        window.render_frame(cx);
        window.click("first-run-create", cx);
    })
    .unwrap();
    // Argon2id runs on the background executor.
    cx.run_until_parked();
}

/// UI 13.1 with D29: the warning is always there, "Remember on this device"
/// is on by default, there is no recovery key step; a password of 8
/// characters creates the vault and the empty Hosts screen opens. The
/// device key went to the (in-memory) secret store.
#[gpui_kit::test]
fn first_run_creates_the_vault_and_opens_the_empty_hosts_screen(cx: &mut TestAppContext) {
    setup(cx);
    let (opened, _) = TestProfile::new().into_opened();
    let core = opened.core.clone();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let (handle, root) = open(&state, cx);
    // Exactly 8 characters, generated (never a fixed test value).
    let password: String = shellpp_desktop::profile::random_suffix()
        .unwrap()
        .chars()
        .take(8)
        .collect();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::FirstRun);
        assert!(visible(window, "first-run-warning"));
        if let Some(on) = window.find("first-run-remember").checked() {
            assert!(on, "Remember on this device is on by default");
        }
        assert!(
            !visible(window, "recovery-key"),
            "no recovery key in local mode"
        );
    })
    .unwrap();

    create_vault(handle, &password, &password, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(state.read(cx).is_unlocked());
        assert_eq!(root.read(cx).screen(cx), Screen::Shell);
        assert!(window.find("hosts-empty").visible());
        assert!(!visible(window, "recovery-key"));
    })
    .unwrap();
    let status = core.lock_status();
    assert_eq!(status.state, LockState::Unlocked);
    assert!(
        status.remember_device_enabled,
        "the device key is stored (in memory in tests)"
    );
    drop(core);
    state.update(cx, |state, _| state.shutdown());
}

/// D29: 7 characters are refused with a message under the field, and two
/// different passwords are refused too; nothing is created.
#[gpui_kit::test]
fn first_run_refuses_short_and_different_passwords(cx: &mut TestAppContext) {
    setup(cx);
    let (opened, _) = TestProfile::new().into_opened();
    let core = opened.core.clone();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let (handle, root) = open(&state, cx);
    let random = shellpp_desktop::profile::random_suffix().unwrap();
    let seven: String = random.chars().take(7).collect();

    create_vault(handle, &seven, &seven, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::FirstRun);
        assert!(visible(window, "first-run-password-error"));
    })
    .unwrap();
    assert_eq!(core.lock_status().state, LockState::FirstRun);

    // Fresh fields: clear both, then two different passwords.
    cx.update_window(handle, |_, window, cx| {
        for id in ["first-run-password", "first-run-repeat"] {
            window.click(id, cx);
            window.press("secondary-a", cx);
            window.press("backspace", cx);
        }
    })
    .unwrap();
    create_vault(handle, &random, &format!("{random}x"), cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::FirstRun);
        assert!(visible(window, "first-run-repeat-error"));
    })
    .unwrap();
    assert_eq!(core.lock_status().state, LockState::FirstRun);
    drop(core);
    state.update(cx, |state, _| state.shutdown());
}

/// UI 13.2: a wrong password shows an error and keeps the lock (the field
/// keeps the focus); the right one opens the shell.
#[gpui_kit::test]
fn unlock_screen_rejects_a_wrong_password(cx: &mut TestAppContext) {
    setup(cx);
    let mut profile = TestProfile::new();
    let password = profile.password().expose_secret().to_owned();
    let opened = profile.open_unlocked();
    opened.core.lock().unwrap();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let (handle, root) = open(&state, cx);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::Unlock);
        assert_eq!(window.find("unlock-password").focused(), Some(true));
        window.input("not-the-password", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("unlock-error").visible());
        // The field keeps the focus (its value is masked from queries).
        assert_eq!(window.find("unlock-password").focused(), Some(true));
        assert_eq!(root.read(cx).screen(cx), Screen::Unlock);
        window.input(&password, cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::Shell);
    })
    .unwrap();
    state.update(cx, |state, _| state.shutdown());
    drop(profile);
}

/// UI 13.3 with lead decision 8: Ctrl+Shift+L locks; open session tabs stay
/// alive but nothing of them is shown behind the unlock screen; after the
/// unlock the same tabs are back, the active one still active.
#[gpui_kit::test]
fn locking_hides_open_sessions_until_unlock(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let password = profile.password().expose_secret().to_owned();
    let ui = Ui::launch(cx, &mut profile);
    let shell = ui.shell(cx);
    let session = shell.update(cx, |shell, cx| {
        shell.open_session(
            shellpp_desktop::shell::SessionRequest::new(shellpp_desktop::shell::SessionKind::Local),
            cx,
        )
    });
    let tab_id = cx.update(|cx| {
        shell
            .read(cx)
            .sessions()
            .get(session)
            .expect("the tab")
            .element_id()
    });
    ui.frame(cx, |window, _| assert!(visible(window, tab_id.clone())));

    ui.frame(cx, |window, cx| window.press("ctrl-shift-l", cx));
    ui.settle(cx);
    assert_eq!(ui.screen(cx), Screen::Unlock, "Ctrl+Shift+L locks");
    ui.frame(cx, |window, _| {
        assert!(visible(window, "unlock"));
        assert!(!visible(window, tab_id.clone()), "the tab is hidden");
        assert!(!visible(window, "shell"), "nothing of the shell is shown");
    });
    // Still alive behind the lock.
    cx.update(|cx| {
        assert!(shell.read(cx).sessions().get(session).is_some());
    });

    ui.frame(cx, |window, cx| {
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.press("enter", cx);
    });
    ui.settle(cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    let same_shell = ui.shell(cx);
    cx.update(|cx| {
        let shell = same_shell.read(cx);
        assert!(shell.sessions().get(session).is_some(), "the tab is back");
        assert_eq!(shell.top_tab(), TopTab::Session(session), "still active");
    });
    ui.frame(cx, |window, _| assert!(visible(window, tab_id.clone())));
    ui.close(cx);
}
