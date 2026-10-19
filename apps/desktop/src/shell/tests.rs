//! Headless tests of the window frame (GPUI `TestAppContext`, no real
//! window, a temporary profile per test): top bar, tabs, sidebar, toasts,
//! notification center, details panel container, exit confirmation.
//! Elements are found by element id (UI 0).

use std::time::Duration;

use gpui::{
    AnyWindowHandle, Bounds, Entity, Pixels, SharedString, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};

use gpui_kit::test::TestWindowExt as _;
use serde_json::json;

use super::*;
use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;
use crate::theme::{Appearance, metrics};

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
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
        ui.frame(cx);
        ui
    }

    fn shell(&self, cx: &mut TestAppContext) -> Entity<Shell> {
        cx.update(|cx| self.root.read(cx).shell().cloned().expect("shell"))
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.frame(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.frame(cx);
    }

    fn bounds(&self, id: &str, cx: &mut TestAppContext) -> Option<Bounds<Pixels>> {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .filter(|element| element.visible())
                .map(|element| element.bounds())
        })
        .unwrap()
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        self.bounds(id, cx).is_some()
    }

    fn top_tab(&self, cx: &mut TestAppContext) -> TopTab {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).top_tab())
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// UI 1.1: the bar is 44 px; the persistent tabs switch the content; the
/// SFTP tab and the sections without a screen show the placeholder.
#[gpui_kit::test]
fn top_bar_tabs_and_placeholders(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let main = ui.bounds("main-area", cx).expect("main area");
    assert_eq!(main.origin.y, metrics::HEIGHT_TOP_BAR, "top bar height");
    for id in [
        "app-menu",
        "tab-vaults",
        "vault-switcher",
        "tab-sftp",
        "tab-new",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert!(ui.visible("top-bar-notifications", cx));
    // The terminal sidebar button only on a session tab (UI 4.2).
    assert!(!ui.visible("terminal-sidebar", cx));

    ui.click("tab-sftp", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    assert!(ui.visible("sftp-placeholder", cx));
    assert!(!ui.visible("main-area", cx));

    // Ctrl+1 / Ctrl+2 (UI 1.4).
    ui.press("ctrl-1", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.press("ctrl-2", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    ui.click("tab-vaults", cx);
    assert!(ui.visible("hosts-screen", cx));
    ui.finish(cx);
}

/// UI 1.2: six sections, 220 px, no vault header, placeholders.
#[gpui_kit::test]
fn sidebar_sections(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let sidebar = ui.bounds("sidebar", cx).expect("sidebar");
    assert_eq!(sidebar.size.width, metrics::WIDTH_SIDEBAR);
    let mut last_y = Pixels::ZERO;
    for section in Section::ALL {
        let item = ui
            .bounds(section.element_id().as_ref(), cx)
            .expect("sidebar item");
        assert!(item.origin.y > last_y, "{section:?} below the previous one");
        last_y = item.origin.y;
    }
    // The first item sits at the top of the sidebar: no vault header above it.
    let hosts = ui.bounds("nav-hosts", cx).unwrap();
    assert_eq!(hosts.origin.y - sidebar.origin.y, metrics::SPACE_2);
    assert_eq!(hosts.size.height, metrics::HEIGHT_CONTROL);

    ui.click("nav-keychain", cx);
    let shell = ui.shell(cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Keychain);
    assert!(ui.visible("section-placeholder", cx));
    assert!(!ui.visible("hosts-screen", cx));
    ui.click("nav-hosts", cx);
    assert!(ui.visible("hosts-screen", cx));
    ui.finish(cx);
}

/// UI 1.1, 4.6: "+", Terminal and Serial open placeholder session tabs; the
/// active tab closes to its neighbor; the last one closes to Vaults.
#[gpui_kit::test]
fn session_tabs_open_and_close(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.click("tab-new", cx);
    let shell = ui.shell(cx);
    let first = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(first.kind(), SessionKind::NewTab);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first.id()));
    assert!(ui.visible(first.element_id().as_ref(), cx));
    assert!(ui.visible("session-placeholder", cx));
    // No sidebar on a session tab (UI 4.1); the terminal sidebar button is there.
    assert!(!ui.visible("sidebar", cx));
    assert!(ui.visible("terminal-sidebar", cx));

    // Ctrl+L: a local terminal tab (placeholder), Ctrl+Alt+S: serial.
    ui.press("ctrl-l", cx);
    ui.press("ctrl-alt-s", cx);
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    assert_eq!(
        tabs.iter().map(SessionTab::kind).collect::<Vec<_>>(),
        [SessionKind::NewTab, SessionKind::Local, SessionKind::Serial]
    );
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));

    // Alt+Left / Alt+Right move between tabs; Ctrl+3 is the first session tab.
    ui.press("alt-left", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[1].id()));
    ui.press("ctrl-3", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));

    // Closing the active middle tab activates its left neighbor.
    ui.click(tabs[1].element_id().as_ref(), cx);
    ui.click(tabs[1].close_element_id().as_ref(), cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));
    // Ctrl+Shift+W closes the active tab: no left neighbor, the right one.
    ui.press("ctrl-shift-w", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));
    ui.press("ctrl-shift-w", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    assert!(cx.update(|cx| shell.read(cx).sessions().is_empty()));
    ui.finish(cx);
}

/// Owner's item 13, UI 1.5: a toast sits at the bottom center of the main
/// area, never over the details panel, and goes away after 5 s; `notify`
/// also lands in the notification center, opening it marks it read.
#[gpui_kit::test]
fn toasts_and_notification_center(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                json!({"address": "192.0.2.10"})
                    .as_object()
                    .cloned()
                    .unwrap(),
                cx,
            )
            .unwrap()
    });
    let shell = ui.shell(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_host_panel(Some(host.record_id), window, cx)
        });
        notify("Upload finished", window, cx);
    })
    .unwrap();
    ui.frame(cx);
    ui.frame(cx);

    let toast = ui.bounds("toast-1", cx).expect("toast");
    let main = ui.bounds("main-area", cx).expect("main area");
    let panel = ui.bounds("right-panel", cx).expect("details panel");
    assert!(
        toast.right() <= panel.left(),
        "the toast does not cover the panel"
    );
    assert!(main.contains(&toast.origin) && main.contains(&toast.bottom_right()));
    // Centered in the main area, 16 px above its bottom edge.
    let center = toast.center().x - main.center().x;
    assert!(center.abs() <= px(1.), "centered: {center:?}");
    assert_eq!(main.bottom() - toast.bottom(), metrics::SPACE_4);

    // The unread counter, then opening the list marks everything read.
    assert!(ui.visible("notification-count", cx));
    ui.click("top-bar-notifications", cx);
    assert!(ui.visible("notifications-list", cx));
    let unread = cx.update(|cx| Notifications::global(cx).read(cx).unread());
    assert_eq!(unread, 0);
    assert!(!ui.visible("notification-count", cx));

    // The toast is gone after 5 s.
    cx.executor().advance_clock(Duration::from_secs(6));
    ui.frame(cx);
    assert!(!ui.visible("toast-1", cx));
    let list = cx.update(|cx| Notifications::global(cx).read(cx).entries().len());
    assert_eq!(list, 1, "the notification stays in the list");
    ui.finish(cx);
}

/// A toast action runs its handler and closes the toast.
#[gpui_kit::test]
fn toast_action_runs_and_closes(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let clicked = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = clicked.clone();
    cx.update_window(ui.handle, move |_, window, cx| {
        show_toast(
            Toast::new("Save host?")
                .action_with_id("toast-save-host", "Save", move |_, _| flag.set(true)),
            window,
            cx,
        );
    })
    .unwrap();
    ui.frame(cx);
    assert!(ui.visible("toast-save-host", cx));
    ui.click("toast-save-host", cx);
    assert!(clicked.get());
    assert!(!ui.visible("toast-1", cx));
    ui.finish(cx);
}

/// UI 1.3: the details panel is 380 px, narrows the main area, and its
/// width follows the drag handle within 320-560 px.
#[gpui_kit::test]
fn details_panel_container(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let full = ui.bounds("main-area", cx).unwrap();
    let shell = ui.shell(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| shell.open_host_panel(None, window, cx));
    })
    .unwrap();
    ui.frame(cx);
    let panel = ui.bounds("right-panel", cx).expect("panel");
    assert_eq!(panel.size.width, metrics::WIDTH_DETAILS);
    let main = ui.bounds("main-area", cx).unwrap();
    assert_eq!(main.size.width, full.size.width - metrics::WIDTH_DETAILS);
    assert_eq!(main.right(), panel.left(), "pushes, never overlaps");

    // Dragging the left edge 60 px to the left widens the panel by 60 px.
    let handle = ui.bounds("panel-resize", cx).expect("resize handle");
    let start = handle.center();
    cx.update_window(ui.handle, move |_, window, cx| {
        window.drag(start, point(start.x - px(60.), start.y), cx);
    })
    .unwrap();
    ui.frame(cx);
    let width = cx.update(|cx| shell.read(cx).details_width());
    assert!(
        (width - px(440.)).abs() <= px(1.),
        "dragged width {width:?}"
    );
    // Limits.
    shell.update(cx, |shell, cx| shell.set_details_width(px(2000.), cx));
    assert_eq!(
        cx.update(|cx| shell.read(cx).details_width()),
        metrics::WIDTH_DETAILS_MAX
    );
    shell.update(cx, |shell, cx| shell.set_details_width(px(10.), cx));
    assert_eq!(
        cx.update(|cx| shell.read(cx).details_width()),
        metrics::WIDTH_DETAILS_MIN
    );
    ui.finish(cx);
}

/// UI 1.4, 4.6: Exit asks first while session tabs are open.
#[gpui_kit::test]
fn exit_asks_while_sessions_are_open(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    shell.update(cx, |shell, cx| {
        shell.open_session(SessionRequest::new(SessionKind::Local), cx);
    });
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| shell.request_exit(window, cx));
    })
    .unwrap();
    ui.frame(cx);
    let asked = cx
        .update_window(ui.handle, |_, window, cx| window.has_active_dialog(cx))
        .unwrap();
    assert!(asked, "the exit confirmation is open");
    ui.finish(cx);
}

/// Placeholder actions show an "In development" toast (audit decision 1).
#[gpui_kit::test]
fn settings_shortcut_shows_the_placeholder(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.press("ctrl-,", cx);
    let shell = ui.shell(cx);
    let messages: Vec<String> = cx.update(|cx| {
        shell
            .read(cx)
            .toasts()
            .read(cx)
            .items()
            .iter()
            .map(|toast| toast.message().to_string())
            .collect()
    });
    assert_eq!(messages, [i18n::placeholder::feature(i18n::menu::SETTINGS)]);
    ui.finish(cx);
}

/// UI 1.4: an app menu item dispatches its action to the shell (keyboard:
/// open the menu, Down selects "Vaults", Enter runs it).
#[gpui_kit::test]
fn app_menu_items_reach_the_shell(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.click("tab-sftp", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    ui.click("app-menu", cx);
    ui.press("down", cx);
    ui.press("enter", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.finish(cx);
}

/// Design-system 6: the tabs of the top bar and the sidebar items are
/// keyboard focus stops (Tab), and Enter on a focused item activates it.
#[gpui_kit::test]
fn tabs_and_sidebar_items_take_keyboard_focus(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let focused = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(ui.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    };
    let mut reached = (false, false);
    for _ in 0..40 {
        ui.press("tab", cx);
        reached.0 |= focused("tab-sftp", cx);
        if focused("nav-keychain", cx) {
            reached.1 = true;
            break;
        }
    }
    assert!(reached.0, "the SFTP tab is a focus stop");
    assert!(reached.1, "the Keychain item is a focus stop");
    ui.press("enter", cx);
    let shell = ui.shell(cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Keychain);
    ui.finish(cx);
}
