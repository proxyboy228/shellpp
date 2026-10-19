//! Headless UI tests of the skeleton (GPUI `TestAppContext`, no real window):
//! Headless UI tests of the lock screens (GPUI `TestAppContext`, no real
//! window): the first run leads to the shell, the unlock screen rejects a
//! wrong password. Every test uses its own temporary profile (`TestProfile`).
//! The Hosts screen and the host panel are tested in `hosts_screen.rs` and
//! `host_panel.rs` (shared helpers in `common/mod.rs`).

mod common;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use shellpp_desktop::app::root::Screen;
use shellpp_desktop::app::{self, RootView};
use shellpp_desktop::state::AppState;
use shellpp_desktop::testing::TestProfile;

use common::setup;

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

/// UI 13.1 (minimal screen): password twice, Continue, the recovery key,
/// the checkbox, Finish, then the shell with the empty Hosts screen.
#[gpui_kit::test]
async fn first_run_leads_to_the_shell(cx: &mut TestAppContext) {
    setup(cx);
    let (opened, password) = TestProfile::new().into_opened();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let (handle, root) = open(&state, cx);
    let password = secrecy::ExposeSecret::expose_secret(&password).to_owned();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::FirstRun);
        assert!(window.find("first-run-warning").visible());
        window.click("first-run-password", cx);
        window.input(&password, cx);
        window.click("first-run-confirm", cx);
        window.input(&password, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("first-run-continue", cx);
    })
    .unwrap();
    // Argon2id runs on the background executor.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(state.read(cx).is_unlocked());
        // Still on the first-run screen: the recovery key step.
        assert_eq!(root.read(cx).screen(cx), Screen::FirstRun);
        assert!(window.find("recovery-key").visible());
        window.click("recovery-saved", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("first-run-finish", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::Shell);
        assert!(window.find("hosts-empty").visible());
    })
    .unwrap();
    state.update(cx, |state, _| state.shutdown());
}

/// UI 13.2 (minimal screen): a wrong password shows an error and keeps the
/// lock; the right one opens the shell.
#[gpui_kit::test]
fn unlock_screen_rejects_a_wrong_password(cx: &mut TestAppContext) {
    setup(cx);
    let mut profile = TestProfile::new();
    let password = secrecy::ExposeSecret::expose_secret(profile.password()).to_owned();
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
