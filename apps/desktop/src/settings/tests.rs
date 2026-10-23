//! Headless tests of the Settings window (UI 12.1, 13.4; decision D29):
//! temporary profiles and the in-memory secret store only.

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, SharedString, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;

use crate::app::{self, RootView};
use crate::state::AppState;
use crate::state::security::AUTO_LOCK_CHOICES;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext, opened: crate::state::OpenedCore) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(opened, cx));
        // The smallest window (960 x 600, lead decision 4).
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(960.), px(600.)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| app::open_window_with(state.clone(), options, cx))
            .expect("test window");
        let ui = Self {
            state,
            handle,
            _root: root,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn bounds(&self, id: &str, cx: &mut TestAppContext) -> Option<gpui::Bounds<gpui::Pixels>> {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .filter(|el| el.visible())
                .map(|el| el.bounds())
        })
        .unwrap()
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        self.bounds(id, cx).is_some()
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

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.settle(cx);
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// UI 12.1 / 13.4: Ctrl+, opens Settings on Security; auto-lock is Never by
/// default and offers Never, 5, 15, 30 and 60 minutes; the choice is
/// stored in the core at once. The window fits the smallest app window.
#[gpui::test]
fn auto_lock_is_never_by_default_and_changes_at_once(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings", cx));
    assert!(ui.visible("settings-security", cx));
    let settings = ui.bounds("settings", cx).unwrap();
    assert!(settings.right() <= px(960.) && settings.bottom() <= px(600.));
    assert_eq!(cx.update(|cx| ui.state.read(cx).auto_lock_minutes()), None);
    assert_eq!(
        AUTO_LOCK_CHOICES,
        [None, Some(5), Some(15), Some(30), Some(60)]
    );

    ui.click("settings-auto-lock", cx);
    for id in [
        "settings-auto-lock-never",
        "settings-auto-lock-5",
        "settings-auto-lock-15",
        "settings-auto-lock-30",
        "settings-auto-lock-60",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert!(!ui.visible("settings-auto-lock-1", cx));
    ui.click("settings-auto-lock-15", cx);
    assert!(!ui.visible("settings-auto-lock-list", cx));
    assert_eq!(
        cx.update(|cx| ui.state.read(cx).auto_lock_minutes()),
        Some(15)
    );
    let stored = cx.update(|cx| {
        ui.state.read(cx).core().unwrap().settings_get().unwrap()["security.auto_lock_minutes"]
            .clone()
    });
    assert_eq!(stored, serde_json::json!(15));
    ui.click("settings-auto-lock", cx);
    ui.click("settings-auto-lock-never", cx);
    assert_eq!(cx.update(|cx| ui.state.read(cx).auto_lock_minutes()), None);

    // Esc closes the window.
    ui.press("escape", cx);
    assert!(!ui.visible("settings", cx));
    ui.finish(cx);
}

/// UI 13.4: "Remember on this device" turns the device key on (with the
/// warning) and off again; the in-memory secret store holds it meanwhile.
#[gpui::test]
fn remember_on_this_device_toggles_the_device_key(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let core = opened.core.clone();
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings-remember", cx));
    assert!(!core.lock_status().remember_device_enabled);

    ui.click("settings-remember", cx);
    assert!(core.lock_status().remember_device_enabled);
    let accounts = profile.memory().unwrap().secret_store.accounts();
    assert_eq!(accounts.len(), 1);
    assert!(ui.visible("settings-remember-warning", cx));

    ui.click("settings-remember", cx);
    assert!(!core.lock_status().remember_device_enabled);
    assert!(profile.memory().unwrap().secret_store.accounts().is_empty());
    assert!(!ui.visible("settings-remember-warning", cx));
    drop(core);
    ui.finish(cx);
    drop(profile);
}

/// UI 13.3: Ctrl+Shift+L locks while the Settings window has the focus too;
/// the window closes with the lock and the unlock screen shows.
#[gpui::test]
fn lock_shortcut_works_while_settings_is_open(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings", cx));
    ui.press("ctrl-shift-l", cx);
    assert!(!cx.update(|cx| ui.state.read(cx).is_unlocked()));
    assert!(!ui.visible("settings", cx));
    assert!(ui.visible("unlock", cx));
    // On the unlock screen the shortcut does nothing.
    ui.press("ctrl-shift-l", cx);
    assert!(ui.visible("unlock", cx));
    ui.finish(cx);
}

/// Without an OS keychain the toggle is hidden; other sections show the
/// "In development" placeholder.
#[gpui::test]
fn sections_and_a_missing_keychain(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    profile.memory().unwrap().secret_store.set_available(false);
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings-auto-lock", cx));
    assert!(!ui.visible("settings-remember", cx));
    // SSH ID and Team stay hidden until accounts and teams (D67).
    for section in super::Section::ALL {
        assert_eq!(
            ui.visible(section.nav_id(), cx),
            section.is_shown(),
            "{section:?}"
        );
    }
    // Every section shown has its content (Updates last, D97).
    ui.click("settings-nav-updates", cx);
    assert!(ui.visible("settings-updates", cx));
    assert!(!ui.visible("settings-placeholder", cx));
    assert!(!ui.visible("settings-security", cx));
    ui.click("settings-nav-security", cx);
    assert!(ui.visible("settings-security", cx));
    ui.finish(cx);
    drop(profile);
}

/// SFTP and Logs (UI 12.6, D67): changes are written at once; Clear the log
/// asks first and removes the entries; Account & Sync offers the data
/// dialogs and a disabled sync server.
#[gpui::test]
fn sftp_and_logs_settings_are_written(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    let core = ui.core(cx);
    let value = |key: &str| core.settings_get().unwrap().get(key).cloned();
    ui.click("settings-nav-sftp", cx);
    assert!(ui.visible("settings-sftp", cx));
    ui.click("settings-sftp-hidden", cx);
    assert_eq!(value("sftp.show_hidden"), Some(serde_json::json!(true)));
    ui.click("settings-sftp-conflict-skip", cx);
    assert_eq!(value("sftp.name_conflict"), Some(serde_json::json!("skip")));
    assert!(reveal(&ui, "settings-sftp-parallel-6", cx));
    ui.click("settings-sftp-parallel-6", cx);
    assert_eq!(value("sftp.parallel_transfers"), Some(serde_json::json!(6)));

    ui.click("settings-nav-logs", cx);
    ui.click("settings-logs-retention-30", cx);
    assert_eq!(value("logs.retention_days"), Some(serde_json::json!(30)));
    // Recording asks first (D75); Cancel leaves it off, Record turns it on,
    // off needs no question.
    assert!(reveal(&ui, "settings-logs-record", cx));
    ui.click("settings-logs-record", cx);
    assert!(ui.visible("settings-record-dialog", cx));
    ui.click("settings-record-cancel", cx);
    assert_ne!(value("logs.record_output"), Some(serde_json::json!(true)));
    ui.click("settings-logs-record", cx);
    ui.click("settings-record-confirm", cx);
    assert_eq!(value("logs.record_output"), Some(serde_json::json!(true)));
    ui.click("settings-logs-record", cx);
    assert!(!ui.visible("settings-record-dialog", cx));
    assert_eq!(value("logs.record_output"), Some(serde_json::json!(false)));
    let started = core
        .connection_log_start(shellpp_core::service::host_api::ConnectionLogStart {
            host: None,
            host_label: None,
            address: "203.0.113.9".into(),
            port: 22,
            username: None,
            os: None,
        })
        .unwrap();
    core.connection_log_end(
        started,
        shellpp_core::service::host_api::ConnectionEndReason::Closed,
        None,
    );
    assert!(reveal(&ui, "settings-clear-log", cx));
    ui.click("settings-clear-log", cx);
    assert!(ui.visible("settings-clear-log-dialog", cx));
    ui.click("settings-clear-log-confirm", cx);
    assert!(core.connection_log_list().unwrap().is_empty());
    assert!(reveal(&ui, "settings-log-cleared", cx));

    ui.click("settings-nav-account", cx);
    assert!(ui.visible("settings-account", cx));
    assert!(ui.visible("settings-export-data", cx));
    ui.finish(cx);
    drop(profile);
}

/// Scrolls the settings section until `id` is on screen (the section is
/// 420 px tall and scrolls); top first.
fn reveal(ui: &Ui, id: &str, cx: &mut TestAppContext) -> bool {
    use gpui::{ScrollDelta, point};
    // Wholly inside the scrolling section (or outside it, like the nav).
    let on_screen =
        |cx: &mut TestAppContext| match (ui.bounds(id, cx), ui.bounds("settings-section", cx)) {
            (Some(target), Some(section)) => {
                !section.intersects(&target)
                    || (target.top() >= section.top() && target.bottom() <= section.bottom())
            }
            _ => false,
        };
    for delta in [2000., -60.] {
        for _ in 0..80 {
            if on_screen(cx) {
                return true;
            }
            cx.update_window(ui.handle, move |_, window, cx| {
                window.scroll(
                    "settings-section",
                    ScrollDelta::Pixels(point(px(0.), px(delta))),
                    cx,
                )
            })
            .unwrap();
            ui.settle(cx);
            if delta > 0. {
                break;
            }
        }
    }
    on_screen(cx)
}

/// Settings > Updates (UI 12.7, D97): the check at launch is off until
/// turned on; the source takes only an https:// address (another text is
/// marked and not written) and Reset to Default clears it. No check runs
/// here: no request leaves the test.
#[gpui::test]
fn updates_settings_are_written(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-updates", cx);
    assert!(ui.visible("settings-updates", cx));
    assert_eq!(
        stored(&ui, "updates.auto_check", cx),
        serde_json::json!(false)
    );
    ui.click("settings-updates-auto", cx);
    assert_eq!(
        stored(&ui, "updates.auto_check", cx),
        serde_json::json!(true)
    );

    type_into(
        &ui,
        "settings-updates-url",
        "http://example.com/latest.json",
        cx,
    );
    assert_eq!(
        stored(&ui, "updates.manifest_url", cx),
        serde_json::Value::Null
    );
    type_into(
        &ui,
        "settings-updates-url",
        "https://example.com/latest.json",
        cx,
    );
    assert_eq!(
        stored(&ui, "updates.manifest_url", cx),
        serde_json::json!("https://example.com/latest.json")
    );
    click(&ui, "settings-updates-reset", cx);
    assert_eq!(
        stored(&ui, "updates.manifest_url", cx),
        serde_json::Value::Null
    );
    assert!(!ui.visible("settings-updates-status", cx), "no check ran");
    ui.finish(cx);
    drop(profile);
}

/// Types `text` into the field with element id `id` (its text replaced).
fn type_into(ui: &Ui, id: &str, text: &str, cx: &mut TestAppContext) {
    click(ui, id, cx);
    ui.press("ctrl-a", cx);
    let text = text.to_owned();
    cx.update_window(ui.handle, move |_, window, cx| window.input(&text, cx))
        .unwrap();
    ui.settle(cx);
}

fn stored(ui: &Ui, key: &str, cx: &mut TestAppContext) -> serde_json::Value {
    cx.update(|cx| ui.state.read(cx).core().unwrap().settings_get().unwrap()[key].clone())
}

/// UI 12.4 (lead decision 12): the Terminal section shows the defaults and
/// writes each change to the core at once; the global prefs follow.
#[gpui::test]
fn terminal_section_defaults_and_changes(cx: &mut TestAppContext) {
    use super::terminal::{BuiltinTheme, TerminalPrefs};
    use serde_json::json;

    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    click(&ui, "settings-nav-terminal", cx);
    assert!(ui.visible("settings-terminal", cx));
    for id in [
        "settings-autoreconnect",
        "settings-record-history",
        "settings-copy-on-select",
        "settings-bell-sound",
        "settings-bold-bright",
        "settings-term",
        "settings-font",
        "settings-font-size",
        "settings-font-size-minus",
        "settings-font-size-plus",
        "settings-theme-graphite",
        "settings-theme-paper",
        "settings-keepalive",
    ] {
        assert!(reveal(&ui, id, cx), "{id}");
    }
    // The blocks this stage does not use stay hidden.
    assert!(!ui.visible("settings-cursor-style", cx));
    let prefs = cx.update(|cx| TerminalPrefs::load(ui.state.read(cx).core().unwrap()));
    assert_eq!(prefs, TerminalPrefs::default());
    assert!(prefs.autoreconnect && !prefs.bell_sound);

    // Toggles.
    click(&ui, "settings-autoreconnect", cx);
    assert_eq!(stored(&ui, "terminal.autoreconnect", cx), json!(false));
    // Record command history: on by default; the core reads the stored value.
    assert!(prefs.record_history);
    click(&ui, "settings-record-history", cx);
    assert_eq!(stored(&ui, "terminal.record_history", cx), json!(false));
    assert!(!cx.update(|cx| TerminalPrefs::current(cx).record_history));
    // The open terminals follow at once (their settings global).
    let terminals =
        |cx: &mut TestAppContext| cx.update(|cx| crate::terminal::settings::settings(cx));
    click(&ui, "settings-bell-sound", cx);
    assert_eq!(stored(&ui, "terminal.bell_sound", cx), json!(true));
    click(&ui, "settings-copy-on-select", cx);
    assert_eq!(stored(&ui, "terminal.copy_on_select", cx), json!(true));
    assert!(terminals(cx).copy_on_select);
    assert!(terminals(cx).bell_sound);

    // TERM: the four choices, xterm-256color by default.
    click(&ui, "settings-term", cx);
    for term in ["xterm", "xterm-256color", "vt100", "linux"] {
        assert!(reveal(&ui, &format!("settings-term-{term}"), cx), "{term}");
    }
    click(&ui, "settings-term-vt100", cx);
    assert!(!ui.visible("settings-term-list", cx));
    assert_eq!(stored(&ui, "terminal.term", cx), json!("vt100"));

    // Font: the bundled font is listed (the test platform has no fonts).
    click(&ui, "settings-font", cx);
    assert!(reveal(&ui, "settings-font-JetBrains Mono", cx));
    click(&ui, "settings-font-JetBrains Mono", cx);
    assert_eq!(
        stored(&ui, "terminal.font_family", cx),
        json!("JetBrains Mono")
    );

    // Font size: -/+ steps, a typed value, an invalid one is not saved.
    click(&ui, "settings-font-size-plus", cx);
    assert_eq!(stored(&ui, "terminal.font_size", cx), json!(15));
    click(&ui, "settings-font-size-minus", cx);
    click(&ui, "settings-font-size-minus", cx);
    assert_eq!(stored(&ui, "terminal.font_size", cx), json!(13));
    type_into(&ui, "settings-font-size", "18", cx);
    assert_eq!(stored(&ui, "terminal.font_size", cx), json!(18));
    assert_eq!(terminals(cx).font_size, 18.);
    type_into(&ui, "settings-font-size", "200", cx);
    assert!(ui.visible("settings-font-size-error", cx));
    assert_eq!(stored(&ui, "terminal.font_size", cx), json!(18));

    // Theme.
    click(&ui, "settings-theme-paper", cx);
    assert_eq!(
        stored(&ui, "terminal.default_theme", cx),
        json!({"builtin": "paper"})
    );
    assert_eq!(terminals(cx).default_theme.name, "Paper");

    // Keepalive: 0 turns it off; a negative or too large value is refused.
    type_into(&ui, "settings-keepalive", "0", cx);
    assert_eq!(stored(&ui, "terminal.keepalive_s", cx), json!(0));
    assert!(!ui.visible("settings-keepalive-error", cx));
    type_into(&ui, "settings-keepalive", "-5", cx);
    // The error line can sit just below the scrolled view.
    assert!(reveal(&ui, "settings-keepalive-error", cx));
    type_into(&ui, "settings-keepalive", "99999", cx);
    // The error line can sit just below the scrolled view.
    assert!(reveal(&ui, "settings-keepalive-error", cx));
    assert_eq!(stored(&ui, "terminal.keepalive_s", cx), json!(0));
    type_into(&ui, "settings-keepalive", "45", cx);
    assert!(!ui.visible("settings-keepalive-error", cx));
    assert_eq!(stored(&ui, "terminal.keepalive_s", cx), json!(45));
    let prefs = cx.update(|cx| TerminalPrefs::load(ui.state.read(cx).core().unwrap()));
    assert_eq!(prefs.keepalive_s, 45);
    assert_eq!(prefs.default_theme, BuiltinTheme::PAPER);
    ui.finish(cx);
}

/// Clicks `id` after scrolling it into view.
fn click(ui: &Ui, id: &str, cx: &mut TestAppContext) {
    assert!(reveal(ui, id, cx), "{id} on screen");
    ui.click(id, cx);
}

/// Owner's stage 2b step 0.4 and UI 4.2: "Adjust indistinguishable colors"
/// offers Off / Palette only / Always (Palette only by default) and the
/// Font block has the line spacing (1.2 by default, 1.0-2.0, steps of 0.1);
/// both are stored at once and the terminals follow.
#[gpui::test]
fn terminal_contrast_and_line_spacing(cx: &mut TestAppContext) {
    use serde_json::json;

    use crate::terminal::MinContrast;

    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    click(&ui, "settings-nav-terminal", cx);
    let terminals =
        |cx: &mut TestAppContext| cx.update(|cx| crate::terminal::settings::settings(cx));
    assert_eq!(terminals(cx).min_contrast, MinContrast::Palette);
    assert_eq!(terminals(cx).line_height, 1.2);

    click(&ui, "settings-min-contrast", cx);
    for key in ["off", "palette", "always"] {
        assert!(
            reveal(&ui, &format!("settings-min-contrast-{key}"), cx),
            "{key}"
        );
    }
    click(&ui, "settings-min-contrast-always", cx);
    assert!(!ui.visible("settings-min-contrast-list", cx));
    assert_eq!(stored(&ui, "terminal.min_contrast", cx), json!("always"));
    assert_eq!(terminals(cx).min_contrast, MinContrast::Always);
    click(&ui, "settings-min-contrast", cx);
    click(&ui, "settings-min-contrast-off", cx);
    assert_eq!(stored(&ui, "terminal.min_contrast", cx), json!("off"));
    assert_eq!(terminals(cx).min_contrast, MinContrast::Off);

    click(&ui, "settings-line-height-plus", cx);
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(1.3));
    click(&ui, "settings-line-height-minus", cx);
    click(&ui, "settings-line-height-minus", cx);
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(1.1));
    type_into(&ui, "settings-line-height", "1.75", cx);
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(1.75));
    assert!((terminals(cx).line_height - 1.75).abs() < 0.001);
    type_into(&ui, "settings-line-height", "0.5", cx);
    // The error line can sit just below the scrolled view.
    assert!(reveal(&ui, "settings-line-height-error", cx));
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(1.75));
    type_into(&ui, "settings-line-height", "2", cx);
    assert!(!ui.visible("settings-line-height-error", cx));
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(2.0));
    // At the maximum the + button is disabled.
    click(&ui, "settings-line-height-plus", cx);
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(2.0));
    ui.finish(cx);
}

/// Shortcuts (UI 12.5, D67): a click records new keys, which work at once
/// and replace the old ones; shared keys are marked; × unassigns and ↺ and
/// Reset all bring the defaults back.
#[gpui::test]
fn shortcuts_are_recorded_and_bound(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-shortcuts", cx);
    assert!(ui.visible("settings-shortcuts", cx));
    let core = ui.core(cx);
    let overrides = |cx: &mut TestAppContext| {
        let _ = cx;
        core.settings_get()
            .unwrap()
            .get("shortcuts.overrides")
            .cloned()
            .unwrap()
    };
    // Lock moves to Ctrl+Shift+J.
    assert!(reveal(&ui, "shortcut-lock", cx));
    ui.click("shortcut-lock", cx);
    ui.press("ctrl-shift-j", cx);
    assert_eq!(overrides(cx), serde_json::json!({"lock": "ctrl-shift-j"}));
    // Settings on Ctrl+L as well: both are marked as sharing keys.
    assert!(reveal(&ui, "shortcut-open_settings", cx));
    ui.click("shortcut-open_settings", cx);
    ui.press("ctrl-l", cx);
    let changed = overrides(cx);
    assert_eq!(changed["open_settings"], serde_json::json!("ctrl-l"));
    let map = changed.as_object().cloned().unwrap();
    assert_eq!(
        crate::shell::shortcuts::conflicts(&map, crate::shell::shortcuts::Preset::platform()),
        ["new_local_terminal", "open_settings"]
    );
    // Esc cancels a recording; × unassigns; ↺ brings the default back.
    assert!(reveal(&ui, "shortcut-next_tab", cx));
    ui.click("shortcut-next_tab", cx);
    ui.press("escape", cx);
    assert!(ui.visible("settings", cx), "Esc only ended the recording");
    assert!(overrides(cx).get("next_tab").is_none());
    ui.click("shortcut-unassign-next_tab", cx);
    assert_eq!(overrides(cx)["next_tab"], serde_json::Value::Null);
    assert!(reveal(&ui, "shortcut-reset-open_settings", cx));
    ui.click("shortcut-reset-open_settings", cx);
    assert!(overrides(cx).get("open_settings").is_none());

    // The new keys lock; the old ones do not any more.
    ui.press("ctrl-shift-l", cx);
    assert!(cx.update(|cx| ui.state.read(cx).is_unlocked()));
    ui.press("ctrl-shift-j", cx);
    assert!(!cx.update(|cx| ui.state.read(cx).is_unlocked()));
    ui.finish(cx);
}

/// The review of the Shortcuts (D67): unassign then reset brings the default
/// back; a key without a modifier is not recorded; Lock moved to keys that
/// another action had leaves that action its other keys.
#[gpui::test]
fn shortcut_changes_undo_cleanly(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-shortcuts", cx);
    let core = ui.core(cx);
    let overrides = || {
        core.settings_get()
            .unwrap()
            .get("shortcuts.overrides")
            .cloned()
            .unwrap()
    };
    assert!(reveal(&ui, "shortcut-lock", cx));
    // A plain letter is not taken; recording goes on.
    ui.click("shortcut-lock", cx);
    ui.press("t", cx);
    assert_eq!(overrides(), serde_json::json!({}));
    ui.press("escape", cx);
    // Unassigned: Ctrl+Shift+L does nothing; reset: it locks again.
    ui.click("shortcut-unassign-lock", cx);
    assert_eq!(overrides()["lock"], serde_json::Value::Null);
    ui.press("ctrl-shift-l", cx);
    assert!(cx.update(|cx| ui.state.read(cx).is_unlocked()));
    assert!(reveal(&ui, "shortcut-reset-lock", cx));
    ui.click("shortcut-reset-lock", cx);
    assert_eq!(overrides(), serde_json::json!({}));
    ui.press("ctrl-shift-l", cx);
    assert!(!cx.update(|cx| ui.state.read(cx).is_unlocked()));
    ui.finish(cx);
}

/// D76: the set of keys and the terminal's keys in the table: Mac is
/// written as the set, the user's keys stay; a terminal key is recorded and
/// is free of conflicts once moved.
#[gpui::test]
fn the_set_of_keys_and_the_terminal_keys(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-shortcuts", cx);
    assert!(ui.visible("shortcuts-preset", cx));
    let core = ui.core(cx);
    let value = |key: &str| core.settings_get().unwrap().get(key).cloned();
    assert!(reveal(&ui, "shortcuts-preset-mac", cx));
    ui.click("shortcuts-preset-mac", cx);
    assert_eq!(value("shortcuts.preset"), Some(serde_json::json!("mac")));
    ui.click("shortcuts-preset-pc", cx);
    assert_eq!(value("shortcuts.preset"), Some(serde_json::json!("pc")));
    // Copy of the terminal moves to Ctrl+Insert.
    assert!(reveal(&ui, "shortcut-copy", cx));
    ui.click("shortcut-copy", cx);
    ui.press("ctrl-insert", cx);
    let overrides = value("shortcuts.overrides").unwrap();
    assert_eq!(overrides["copy"], serde_json::json!("ctrl-insert"));
    let map = overrides.as_object().cloned().unwrap();
    assert!(
        crate::shell::shortcuts::conflicts(&map, crate::shell::shortcuts::Preset::Pc).is_empty()
    );
    // The Mac set keeps that change.
    assert!(reveal(&ui, "shortcuts-preset-mac", cx));
    ui.click("shortcuts-preset-mac", cx);
    assert_eq!(
        value("shortcuts.overrides").unwrap()["copy"],
        serde_json::json!("ctrl-insert")
    );
    ui.finish(cx);
}

/// Stage 7 (UI 12.2, 13.5): without an account the section offers the
/// connect wizard; it opens on the server step.
#[gpui::test]
fn account_section_opens_the_connect_wizard(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-account", cx);
    assert!(ui.visible("settings-connect-sync", cx));
    ui.click("settings-connect-sync", cx);
    assert!(ui.visible("sync-connect", cx), "the wizard opened");
    assert!(ui.visible("sync-server", cx), "on the server step");
    assert!(ui.visible("sync-check", cx));
    // An empty address is refused before any request.
    ui.click("sync-check", cx);
    assert!(ui.visible("sync-error", cx));
}

/// A bound account (status as the core reports it): email, status, Sync
/// now, the toggles; a toggle writes `sync.toggles` at once.
#[gpui::test]
fn bound_account_shows_status_and_writes_toggles(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let ui = Ui::open(cx, opened);
    cx.update(|cx| {
        ui.state.update(cx, |state, cx| {
            state.set_sync_status(
                shellpp_core::service::SyncStatus {
                    bound: true,
                    signed_in: true,
                    server_url: Some("https://sync.example.com".into()),
                    email: Some("user@example.com".into()),
                    pending: 2,
                    ..Default::default()
                },
                cx,
            )
        })
    });
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-account", cx);
    assert!(ui.visible("sync-block", cx));
    assert!(ui.visible("sync-account-email", cx));
    assert!(ui.visible("sync-now", cx));
    assert!(!ui.visible("settings-connect-sync", cx));
    assert!(reveal(&ui, "sync-toggle-history", cx));
    ui.click("sync-toggle-history", cx);
    assert_eq!(
        stored(&ui, "sync.toggles", cx),
        serde_json::json!({"history": true}),
        "history turned on, the rest keep their defaults"
    );
}

/// UI 13.4 (stage 7): Change master password with "My old password may be
/// compromised" changes the password and gives the vault a new key; the
/// new password unlocks, the old one does not.
#[gpui::test]
fn change_password_with_rotation_from_security(cx: &mut TestAppContext) {
    use secrecy::ExposeSecret as _;
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let old = profile.password().expose_secret().to_owned();
    let ui = Ui::open(cx, opened);
    ui.press("ctrl-,", cx);
    assert!(reveal(&ui, "settings-change-password", cx));
    ui.click("settings-change-password", cx);
    assert!(ui.visible("sync-password-change", cx));
    let new = format!("{old}-new");
    // The fields of the dialog (the Settings window closed: no section to
    // scroll).
    let type_text = |id: &str, text: &str, cx: &mut TestAppContext| {
        ui.click(id, cx);
        let text = text.to_owned();
        cx.update_window(ui.handle, move |_, window, cx| window.input(&text, cx))
            .unwrap();
        ui.settle(cx);
    };
    type_text("sync-current-password", &old, cx);
    type_text("sync-new-password", &new, cx);
    type_text("sync-repeat-password", &new, cx);
    ui.click("sync-compromised", cx);
    ui.click("sync-password-run", cx);
    for _ in 0..50 {
        if ui.visible("sync-password-close", cx) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        ui.settle(cx);
    }
    assert!(ui.visible("sync-password-close", cx), "the change finished");
    let core = ui.core(cx);
    core.lock().unwrap();
    assert!(
        core.unlock(secrecy::SecretString::from(old)).is_err(),
        "the old password no longer opens"
    );
    core.unlock(secrecy::SecretString::from(new))
        .expect("the new password opens");
}
