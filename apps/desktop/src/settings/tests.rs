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
use crate::theme::{self, Appearance};

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext, opened: crate::state::OpenedCore) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
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
    ui.click("settings-nav-terminal", cx);
    assert!(ui.visible("settings-placeholder", cx));
    assert!(!ui.visible("settings-security", cx));
    ui.click("settings-nav-security", cx);
    assert!(ui.visible("settings-security", cx));
    ui.finish(cx);
    drop(profile);
}
