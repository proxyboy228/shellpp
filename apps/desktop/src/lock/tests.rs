//! Headless tests of the lock screens (UI 13.1-13.3, decision D29; GPUI
//! `TestAppContext`, no real window). Every test uses its own temporary
//! profile and the in-memory secret store: never the permanent profile or
//! the OS keychain.

use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, ScrollDelta, SharedString, TestAppContext,
    WindowBounds, WindowOptions, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::ExposeSecret as _;
use serde_json::json;
use shellpp_core::service::dto::LockState;

use super::*;
use crate::app::root::Screen;
use crate::app::{self, RootView};
use crate::shell::{SessionKind, SessionRequest};
use crate::state::AppState;
use crate::state::security::ArchivedDataFile;
use crate::testing::TestProfile;
use crate::theme::{self, Appearance};

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext, opened: crate::state::OpenedCore) -> Self {
        Self::open_sized(cx, opened, 1280., 800.)
    }

    fn open_sized(
        cx: &mut TestAppContext,
        opened: crate::state::OpenedCore,
        width: f32,
        height: f32,
    ) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(opened, cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(height)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| app::open_window_with(state.clone(), options, cx))
            .expect("test window");
        let ui = Self {
            state,
            handle,
            root,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn screen(&self, cx: &mut TestAppContext) -> Screen {
        cx.update(|cx| self.root.read(cx).screen(cx))
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn type_in(&self, id: &str, text: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
            window.input(&text, cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.settle(cx);
    }

    fn unlock_screen(&self, cx: &mut TestAppContext) -> Entity<UnlockScreen> {
        cx.update(|cx| {
            self.root
                .read(cx)
                .unlock_screen()
                .cloned()
                .expect("unlock screen")
        })
    }

    fn first_run(&self, cx: &mut TestAppContext) -> Entity<FirstRunScreen> {
        cx.update(|cx| {
            self.root
                .read(cx)
                .first_run_screen()
                .cloned()
                .expect("first-run screen")
        })
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// A profile that has a vault and is locked again; the profile keeps its
/// in-memory platform (clock, secret store) for the test.
fn locked_profile() -> (TestProfile, crate::state::OpenedCore) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    opened.core.lock().expect("lock");
    (profile, opened)
}

/// UI 13.1: the strength indicator follows the password; "Remember on this
/// device" can be turned off before the vault is created, and then no device
/// key is stored.
#[gpui::test]
fn strength_indicator_and_remember_toggle(cx: &mut TestAppContext) {
    let (opened, password) = TestProfile::new().into_opened();
    let core = opened.core.clone();
    let ui = Ui::open(cx, opened);
    assert_eq!(ui.screen(cx), Screen::FirstRun);
    let screen = ui.first_run(cx);
    assert!(ui.visible("first-run-strength", cx));
    assert!(ui.visible("lock-subtitle", cx));
    cx.update(|cx| {
        assert_eq!(screen.read(cx).strength(), None);
        assert!(screen.read(cx).remember(cx), "on by default");
    });

    ui.type_in("first-run-password", "password", cx);
    cx.update(|cx| assert_eq!(screen.read(cx).strength(), Some(Strength::Weak)));
    // Not enough yet: one field is empty.
    cx.update(|cx| assert!(!screen.read(cx).can_create(cx)));

    // Turn "Remember on this device" off, then create the vault.
    ui.click("first-run-remember", cx);
    cx.update(|cx| assert!(!screen.read(cx).remember(cx)));
    let password = password.expose_secret().to_owned();
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("first-run-password", cx);
        window.press("secondary-a", cx);
        window.input(&password, cx);
        window.click("first-run-repeat", cx);
        window.input(&password, cx);
    })
    .unwrap();
    ui.settle(cx);
    cx.update(|cx| {
        assert!(screen.read(cx).strength() >= Some(Strength::Fair));
        assert!(screen.read(cx).can_create(cx));
    });
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    let status = core.lock_status();
    assert_eq!(status.state, LockState::Unlocked);
    assert!(!status.remember_device_enabled, "turned off at first run");
    drop(core);
    ui.finish(cx);
}

/// UI 13.1: Enter in either field does what "Create vault" does. With an
/// empty "Repeat password" it creates nothing and says why under that field.
#[gpui::test]
fn enter_in_the_first_field_tries_to_create_the_vault(cx: &mut TestAppContext) {
    let (opened, password) = TestProfile::new().into_opened();
    let ui = Ui::open(cx, opened);
    let screen = ui.first_run(cx);
    let password = password.expose_secret().to_owned();
    ui.type_in("first-run-password", &password, cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::FirstRun, "nothing created");
    assert!(ui.visible("first-run-repeat-error", cx));
    assert!(!ui.visible("first-run-password-error", cx), "long enough");
    cx.update(|cx| {
        assert_eq!(
            screen.read(cx).repeat_error(cx),
            Some(strings::PASSWORDS_DIFFER)
        )
    });
    ui.type_in("first-run-repeat", &password, cx);
    cx.update(|cx| assert_eq!(screen.read(cx).repeat_error(cx), None));
    // Enter in the first field now creates the vault.
    ui.click("first-run-password", cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    ui.finish(cx);
}

/// UI 13.1: an attempt with an empty first field says "At least 8
/// characters" under it.
#[gpui::test]
fn an_empty_password_is_too_short_on_an_attempt(cx: &mut TestAppContext) {
    let (opened, _) = TestProfile::new().into_opened();
    let ui = Ui::open(cx, opened);
    assert!(!ui.visible("first-run-password-error", cx));
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::FirstRun);
    assert!(ui.visible("first-run-password-error", cx));
    ui.finish(cx);
}

/// UI 13.1: without an OS keychain the toggle is hidden.
#[gpui::test]
fn remember_is_hidden_without_a_keychain(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open();
    profile
        .memory()
        .expect("memory platform")
        .secret_store
        .set_available(false);
    let ui = Ui::open(cx, opened);
    assert_eq!(ui.screen(cx), Screen::FirstRun);
    assert!(ui.visible("first-run-warning", cx));
    assert!(!ui.visible("first-run-remember", cx));
    let screen = ui.first_run(cx);
    cx.update(|cx| assert!(!screen.read(cx).remember(cx)));
    ui.finish(cx);
    drop(profile);
}

/// UI 13.2, D29: five wrong passwords start a pause; the screen counts it
/// down with Unlock inactive, refuses even the right password meanwhile,
/// and unlocks after the pause.
#[gpui::test]
fn wrong_passwords_pause_unlock_with_a_countdown(cx: &mut TestAppContext) {
    let (profile, opened) = locked_profile();
    let password = profile.password().expose_secret().to_owned();
    let ui = Ui::open(cx, opened);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    let screen = ui.unlock_screen(cx);
    for attempt in 1..=5 {
        ui.type_in("unlock-password", &format!("wrong-{attempt}-password"), cx);
        ui.press("enter", cx);
    }
    assert!(ui.visible("unlock-retry", cx));
    assert!(!ui.visible("unlock-error", cx));
    let left = cx.update(|cx| screen.read(cx).retry_after());
    assert!(matches!(left, Some(1..=5)), "{left:?}");

    // The right password is refused during the pause (Enter does nothing).
    ui.type_in("unlock-password", &password, cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Unlock);

    // The pause ends: the core's clock moves on, the countdown notices it.
    profile
        .memory()
        .expect("memory platform")
        .clock
        .advance(Duration::from_secs(6));
    cx.executor().advance_clock(Duration::from_secs(1));
    ui.settle(cx);
    cx.update(|cx| assert_eq!(screen.read(cx).retry_after(), None));
    assert!(!ui.visible("unlock-retry", cx));
    ui.type_in("unlock-password", &password, cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    ui.finish(cx);
    drop(profile);
}

/// UI 13.2, D29: "Forgot password?" explains, keeps "Restore from sync"
/// inactive, and "Start with a new vault" needs a second confirmation
/// (Enter and Esc cancel). Confirming renames the data file (it is kept)
/// and returns to the first run, which names the kept file.
#[gpui::test]
fn forgot_password_starts_a_new_vault_and_keeps_the_old_file(cx: &mut TestAppContext) {
    let (profile, opened) = locked_profile();
    let data_dir = profile.path().to_path_buf();
    let ui = Ui::open(cx, opened);
    let screen = ui.unlock_screen(cx);

    ui.click("unlock-forgot", cx);
    assert!(ui.visible("forgot-dialog", cx));
    assert!(!ui.visible("forgot-remembered", cx), "nothing remembered");
    // Inactive: a click changes nothing.
    ui.click("forgot-restore-sync", cx);
    assert!(ui.visible("forgot-new-vault", cx));
    ui.press("escape", cx);
    assert!(!ui.visible("forgot-dialog", cx), "Esc closes");
    cx.update(|cx| assert!(screen.read(cx).forgot().is_none()));

    ui.click("unlock-forgot", cx);
    ui.click("forgot-new-vault", cx);
    assert!(ui.visible("forgot-start-over", cx));
    // Enter is Cancel.
    ui.press("enter", cx);
    assert!(!ui.visible("forgot-dialog", cx));
    assert_eq!(ui.screen(cx), Screen::Unlock);

    ui.click("unlock-forgot", cx);
    ui.click("forgot-new-vault", cx);
    ui.click("forgot-start-over", cx);
    assert_eq!(ui.screen(cx), Screen::FirstRun);
    assert!(ui.visible("first-run-archived", cx));
    let archived = cx
        .update(|cx| ArchivedDataFile::get(cx))
        .expect("archived file name");
    assert!(archived.starts_with("shellpp.db.") && archived.ends_with(".old"));
    assert!(data_dir.join(&archived).is_file(), "the old file is kept");
    assert!(data_dir.join("shellpp.db").is_file());

    // A new vault: empty.
    let password = crate::testing::generated_password();
    let password = password.expose_secret().to_owned();
    ui.type_in("first-run-password", &password, cx);
    ui.type_in("first-run-repeat", &password, cx);
    ui.click("first-run-create", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    assert!(ui.visible("hosts-empty", cx));
    assert_eq!(cx.update(|cx| ArchivedDataFile::get(cx)), None);
    ui.finish(cx);
    drop(profile);
}

/// With "Remember on this device" on, the card says that a restart opens
/// the vault (the device key is used at launch only).
#[gpui::test]
fn forgot_password_mentions_the_remembered_key(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    opened.core.remember_device_set(true).expect("remember");
    opened.core.lock().expect("lock");
    let ui = Ui::open(cx, opened);
    ui.click("unlock-forgot", cx);
    assert!(ui.visible("forgot-remembered", cx));
    ui.finish(cx);
    drop(profile);
}

/// UI 13.3: the Lock action closes the details panel and the Settings
/// window and shows the unlock screen; auto-lock from the core does the
/// same; the open session tab survives both.
#[gpui::test]
fn lock_and_auto_lock_keep_sessions_and_close_overlays(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let password = profile.password().expose_secret().to_owned();
    let opened = profile.open_unlocked();
    let core = opened.core.clone();
    let ui = Ui::open(cx, opened);
    // An open group is part of the window's state too.
    let group = ui.state.update(cx, |state, cx| {
        let group = state
            .save_group(
                None,
                json!({"name": "Lab"}).as_object().cloned().unwrap(),
                cx,
            )
            .expect("group")
            .record_id;
        state.set_location(Some(group), cx);
        group
    });
    let [host, _untagged] =
        [("192.0.2.10", vec!["prod"]), ("192.0.2.11", vec![])].map(|(address, tags)| {
            ui.state.update(cx, |state, cx| {
                state
                    .save_host(
                        None,
                        json!({"address": address, "tags": tags, "group_id": group})
                            .as_object()
                            .cloned()
                            .unwrap(),
                        cx,
                    )
                    .expect("host")
                    .record_id
            })
        });
    let shell = cx.update(|cx| ui.root.read(cx).shell().cloned().expect("shell"));
    // The tag filter is part of the window's state as well.
    let hosts = cx.update(|cx| shell.read(cx).hosts_screen().clone());
    hosts.update(cx, |hosts, cx| hosts.toggle_tag("prod", cx));
    ui.settle(cx);
    cx.update(|cx| assert_eq!(hosts.read(cx).visible_host_ids(), vec![host]));
    let session = shell.update(cx, |shell, cx| {
        shell.open_session(SessionRequest::new(SessionKind::Local), cx)
    });
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.set_top_tab(crate::shell::TopTab::Vaults, cx);
            shell.open_host_panel(Some(host), window, cx);
        });
    })
    .unwrap();
    ui.settle(cx);
    assert!(ui.visible("host-panel", cx));
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings", cx));

    // Ctrl+Shift+L locks while the Settings window has the focus too.
    ui.press("ctrl-shift-l", cx);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    assert!(!ui.visible("settings", cx), "the Settings window is closed");
    assert!(!ui.visible("shell", cx));
    cx.update(|cx| {
        assert!(shell.read(cx).panel().is_none(), "the panel is closed");
        assert!(shell.read(cx).sessions().get(session).is_some());
    });

    ui.type_in("unlock-password", &password, cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    // The window returns to its state: the same panel, the Vaults tab.
    cx.update(|cx| {
        let shell = shell.read(cx);
        let panel = match shell.panel() {
            Some(crate::shell::Panel::Host(panel)) => panel.read(cx).record_id(),
            _ => None,
        };
        assert_eq!(panel, Some(host), "the host panel is back");
        assert_eq!(ui.state.read(cx).location(), Some(group));
        assert_eq!(shell.top_tab(), crate::shell::TopTab::Vaults);
        assert!(shell.sessions().get(session).is_some());
        // The tag filter and its listing are back too.
        let hosts = hosts.read(cx);
        assert_eq!(hosts.selected_tags(), ["prod".to_owned()]);
        assert_eq!(hosts.visible_host_ids(), vec![host]);
    });

    // Auto-lock after 5 minutes without input. An edit still waiting for
    // the autosave (here: typed just before a sleep) is saved first.
    ui.state
        .update(cx, |state, cx| state.set_auto_lock_minutes(Some(5), cx))
        .expect("auto-lock");
    ui.type_in("host-label", "Renamed", cx);
    let pending = cx.update(|cx| match shell.read(cx).panel() {
        Some(crate::shell::Panel::Host(panel)) => panel.read(cx).has_pending_save(),
        _ => false,
    });
    assert!(pending, "the edit waits for the autosave");
    profile
        .memory()
        .expect("memory platform")
        .clock
        .advance(Duration::from_secs(5 * 60 + 1));
    // The app's tick asks the windows to lock; they save, then lock.
    assert!(ui.state.update(cx, |state, cx| state.check_auto_lock(cx)));
    ui.settle(cx);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    cx.update(|cx| assert!(shell.read(cx).sessions().get(session).is_some()));
    ui.type_in("unlock-password", &password, cx);
    ui.press("enter", cx);
    assert_eq!(ui.screen(cx), Screen::Shell);
    cx.update(|cx| {
        let host = ui.state.read(cx).host(host).cloned().expect("host");
        assert_eq!(
            host.label.as_deref(),
            Some("Renamed"),
            "saved before the lock"
        );
    });

    // The core's own tick still locks when no window takes the request.
    profile
        .memory()
        .expect("memory platform")
        .clock
        .advance(Duration::from_secs(5 * 60 + 1));
    core.tick();
    ui.settle(cx);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    drop(core);
    ui.finish(cx);
    drop(profile);
}

/// UI 13.3: scrolling is input too. Scroll wheel events alone (no click, no
/// key) keep a 5-minute auto-lock from firing.
#[gpui::test]
fn scrolling_alone_keeps_auto_lock_from_firing(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let core = opened.core.clone();
    let ui = Ui::open(cx, opened);
    ui.state
        .update(cx, |state, cx| state.set_auto_lock_minutes(Some(5), cx))
        .expect("auto-lock");
    let clock = profile.memory().expect("memory platform").clock.clone();
    clock.advance(Duration::from_secs(4 * 60));
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll("main-area", ScrollDelta::Lines(point(0., 3.)), cx);
    })
    .unwrap();
    ui.settle(cx);
    clock.advance(Duration::from_secs(2 * 60));
    assert!(!ui.state.update(cx, |state, cx| state.check_auto_lock(cx)));
    core.tick();
    ui.settle(cx);
    assert_eq!(
        ui.screen(cx),
        Screen::Shell,
        "the scroll restarted the timer"
    );

    // Without input the interval passes.
    clock.advance(Duration::from_secs(3 * 60 + 1));
    assert!(ui.state.update(cx, |state, cx| state.check_auto_lock(cx)));
    ui.settle(cx);
    assert_eq!(ui.screen(cx), Screen::Unlock);
    drop(core);
    ui.finish(cx);
    drop(profile);
}

/// Neither the screens' `Debug` output nor the app state's shows a typed
/// password.
#[gpui::test]
fn debug_output_never_shows_a_typed_password(cx: &mut TestAppContext) {
    let (opened, _) = TestProfile::new().into_opened();
    let ui = Ui::open(cx, opened);
    let canary = format!("Canary-{}", crate::profile::random_suffix().unwrap());
    ui.type_in("first-run-password", &canary, cx);
    ui.type_in("first-run-repeat", &canary, cx);
    let screen = ui.first_run(cx);
    let debug = cx.update(|cx| {
        format!(
            "{:?} {:?} {:?}",
            screen.read(cx),
            ui.root.read(cx),
            ui.state.read(cx)
        )
    });
    assert!(!debug.contains(&canary));
    ui.finish(cx);
}

/// Lead decision 4: at the smallest window (960 x 600) the lock cards and
/// the "Forgot password?" card fit without clipping.
#[gpui::test]
fn lock_screens_fit_the_smallest_window(cx: &mut TestAppContext) {
    let (opened, _) = TestProfile::new().into_opened();
    let ui = Ui::open_sized(cx, opened, 960., 600.);
    let inside = |ui: &Ui, id: &str, cx: &mut TestAppContext| {
        let id = SharedString::from(id.to_owned());
        cx.update_window(ui.handle, move |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find(id.clone()).bounds();
            bounds.origin.y >= px(0.)
                && bounds.bottom() <= px(600.)
                && bounds.origin.x >= px(0.)
                && bounds.right() <= px(960.)
        })
        .unwrap()
    };
    assert_eq!(ui.screen(cx), Screen::FirstRun);
    assert!(inside(&ui, "lock-card", cx), "first-run card");
    assert!(inside(&ui, "first-run-create", cx));
    ui.finish(cx);

    let (profile, opened) = locked_profile();
    let ui = Ui::open_sized(cx, opened, 960., 600.);
    assert!(inside(&ui, "lock-card", cx), "unlock card");
    ui.click("unlock-forgot", cx);
    assert!(inside(&ui, "forgot-dialog", cx));
    ui.click("forgot-new-vault", cx);
    assert!(inside(&ui, "forgot-start-over", cx));
    ui.finish(cx);
    drop(profile);
}
