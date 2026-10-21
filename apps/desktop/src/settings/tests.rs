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
    for section in super::Section::ALL {
        assert!(ui.visible(section.nav_id(), cx), "{section:?}");
    }
    ui.click("settings-nav-sftp", cx);
    assert!(ui.visible("settings-placeholder", cx));
    assert!(!ui.visible("settings-security", cx));
    ui.click("settings-nav-security", cx);
    assert!(ui.visible("settings-security", cx));
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
    assert!(ui.visible("settings-keepalive-error", cx));
    type_into(&ui, "settings-keepalive", "99999", cx);
    assert!(ui.visible("settings-keepalive-error", cx));
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
    assert!(ui.visible("settings-line-height-error", cx));
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(1.75));
    type_into(&ui, "settings-line-height", "2", cx);
    assert!(!ui.visible("settings-line-height-error", cx));
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(2.0));
    // At the maximum the + button is disabled.
    click(&ui, "settings-line-height-plus", cx);
    assert_eq!(stored(&ui, "terminal.line_height", cx), json!(2.0));
    ui.finish(cx);
}
