//! Headless UI tests of the skeleton (GPUI `TestAppContext`, no real window):
//! first run, the shell on an unlocked profile, the host panel autosave.
//! Every test uses its own temporary profile (`TestProfile`).
//!
//! Template for new UI tests: `setup`, a state from `TestProfile`, then
//! `app::open_window_with` with a fixed size, `window.render_frame`, find and
//! click elements by id, `cx.run_until_parked()` between steps, and
//! `AppState::shutdown` at the end (deletes the profile).

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use serde_json::{Map, Value, json};
use shellpp_desktop::app::root::Screen;
use shellpp_desktop::app::{self, RootView};
use shellpp_desktop::hosts::host_card_id;
use shellpp_desktop::shell::Panel;
use shellpp_desktop::state::AppState;
use shellpp_desktop::testing::TestProfile;
use shellpp_desktop::theme::{self, Appearance};

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
        app::root::bind_keys(cx);
    });
}

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

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
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

/// The shell on an unlocked profile: host cards, single click opens the
/// panel, an edit is saved automatically after the debounce.
#[gpui_kit::test]
fn host_card_opens_the_panel_and_edits_autosave(cx: &mut TestAppContext) {
    setup(cx);
    let opened = TestProfile::new().into_unlocked();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let web = state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({"address": "192.0.2.10"})), cx)
            .unwrap();
        state
            .save_host(
                None,
                map(json!({"address": "web1.example.com", "label": "Web"})),
                cx,
            )
            .unwrap()
    });
    let (handle, root) = open(&state, cx);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(root.read(cx).screen(cx), Screen::Shell);
        assert!(window.find(host_card_id(web.record_id)).visible());
        // No autofocus in the search bar (UI 2.1).
        assert_eq!(window.find("search").focused(), Some(false));
        window.click(host_card_id(web.record_id), cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-panel").visible());
        assert_eq!(window.find("host-label").value(), Some("Web"));
        assert_eq!(
            state.read(cx).selection().single(),
            Some(web.record_id),
            "single click selects"
        );
        window.click("host-label", cx);
        window.input(" 1", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Not saved before the 500 ms debounce, saved after it.
    cx.update(|cx| {
        assert_eq!(
            state.read(cx).host(web.record_id).unwrap().label.as_deref(),
            Some("Web")
        );
    });
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            state.read(cx).host(web.record_id).unwrap().label.as_deref(),
            Some("Web 1")
        );
        let shell = root.read(cx).shell().cloned().expect("shell");
        assert!(matches!(shell.read(cx).panel(), Some(Panel::Host(_))));
    });
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
