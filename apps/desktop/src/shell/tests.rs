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
        // Session tabs have terminals that parse on their own threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
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

    /// Delivers the next frame (tests have no frame loop): what the views
    /// asked to run after the last frame runs, then a frame is drawn.
    fn next_frame(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.simulate_next_frame(cx);
        })
        .unwrap();
        self.frame(cx);
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

/// UI 1.1, 4.6, 12.5 (D30): "+" is shown disabled and opens nothing;
/// Ctrl+L outside a terminal opens a local terminal tab and Ctrl+Alt+S a
/// serial placeholder; Alt+Left/Right and Ctrl+N move between tabs; the
/// active tab closes to its neighbor; the last one closes to Vaults.
#[gpui_kit::test]
fn session_tabs_open_and_close(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    assert!(ui.visible("tab-new", cx));
    ui.click("tab-new", cx);
    assert!(
        cx.update(|cx| shell.read(cx).sessions().is_empty()),
        "+ is disabled"
    );

    // Ctrl+L: a local terminal tab, Ctrl+Alt+S: serial (placeholder).
    ui.press("ctrl-l", cx);
    let local = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(local.kind(), SessionKind::Local);
    assert!(cx.update(|cx| shell.read(cx).pane(local.id()).is_some()));
    // No sidebar on a session tab (UI 4.1); the terminal sidebar button is
    // there, disabled (stage 2b).
    assert!(!ui.visible("sidebar", cx));
    assert!(ui.visible("terminal-sidebar", cx));
    assert!(ui.visible("session-pane", cx));
    // Back on Vaults, so that Ctrl+L and Ctrl+Alt+S reach the shell.
    ui.press("ctrl-1", cx);
    ui.press("ctrl-l", cx);
    ui.press("ctrl-alt-s", cx);
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    assert_eq!(
        tabs.iter().map(SessionTab::kind).collect::<Vec<_>>(),
        [SessionKind::Local, SessionKind::Local, SessionKind::Serial]
    );
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));
    assert!(ui.visible("session-placeholder", cx));

    // Alt+Left / Alt+Right move between tabs; Ctrl+3 is the first session tab.
    ui.press("alt-left", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[1].id()));
    ui.press("alt-right", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));
    ui.press("ctrl-3", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));
    ui.press("ctrl-2", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    ui.press("ctrl-3", cx);

    // Closing the active middle tab activates its left neighbor.
    ui.click(tabs[1].element_id().as_ref(), cx);
    ui.click(tabs[1].close_element_id().as_ref(), cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));
    assert!(cx.update(|cx| shell.read(cx).pane(tabs[1].id()).is_none()));
    // Ctrl+Shift+W closes the active tab: no left neighbor, the right one.
    ui.press("ctrl-shift-w", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));
    ui.press("ctrl-shift-w", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    assert!(cx.update(|cx| shell.read(cx).sessions().is_empty()));
    ui.finish(cx);
}

/// UI 1.1 [decision]: more session tabs than fit (ten sessions and more)
/// are not clipped away: the strip scrolls sideways, the active tab always
/// scrolls into view (opened, Ctrl+N, Alt+Left/Right, closed to a
/// neighbor), the wheel over a tab scrolls the strip, the tabs keep their
/// minimum width and "+" stays in the bar.
#[gpui_kit::test]
fn many_session_tabs_scroll_into_view(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    // Serial tabs show the placeholder: no terminals to start. The first
    // one comes from Vaults: the bar changes (the terminal sidebar button).
    for _ in 0..12 {
        shell.update(cx, |shell, cx| {
            shell.open_session(SessionRequest::new(SessionKind::Serial), cx);
        });
    }
    ui.next_frame(cx);
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    let strip = ui.bounds("session-tabs", cx).expect("tab strip");
    // A tab wholly inside the strip (half a pixel for rounding).
    let in_view = |tab: &SessionTab, cx: &mut TestAppContext| {
        ui.bounds(tab.element_id().as_ref(), cx)
            .is_some_and(|bounds| {
                bounds.left() >= strip.left() - px(0.5) && bounds.right() <= strip.right() + px(0.5)
            })
    };
    let left = |tab: &SessionTab, cx: &mut TestAppContext| {
        ui.bounds(tab.element_id().as_ref(), cx)
            .expect("tab in view")
            .left()
    };

    // The newest tab is active and in view; the first one is scrolled away.
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[11].id()));
    assert!(in_view(&tabs[11], cx), "the active tab is in view");
    assert!(!in_view(&tabs[0], cx), "more tabs than fit");
    let width = ui
        .bounds(tabs[11].element_id().as_ref(), cx)
        .unwrap()
        .size
        .width;
    assert!(
        width >= title_bar::SESSION_TAB_MIN_WIDTH - px(0.5),
        "{width:?}"
    );
    // "+" stays in the bar, right of the strip, left of the bell.
    let new_tab = ui.bounds("tab-new", cx).expect("+ in view");
    let bell = ui.bounds("top-bar-notifications", cx).expect("bell");
    assert!(new_tab.left() >= strip.right() && new_tab.right() <= bell.left());

    // Ctrl+3: the first session tab scrolls into view.
    ui.press("ctrl-3", cx);
    ui.next_frame(cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));
    assert!(in_view(&tabs[0], cx));
    assert!(!in_view(&tabs[11], cx));
    // Alt+Left from Vaults wraps to the last tab, which scrolls into view
    // although the strip is narrower than on Vaults.
    ui.press("ctrl-1", cx);
    ui.next_frame(cx);
    ui.press("alt-left", cx);
    ui.next_frame(cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[11].id()));
    assert!(in_view(&tabs[11], cx));
    assert!(!in_view(&tabs[0], cx));

    // The wheel over a tab scrolls the strip (a vertical wheel sideways),
    // and the active tab stays where the wheel left it: partly hidden.
    let before = left(&tabs[11], cx);
    let target = tabs[11].element_id();
    cx.update_window(ui.handle, move |_, window, cx| {
        window.scroll(
            target.clone(),
            gpui::ScrollDelta::Pixels(point(px(0.), px(30.))),
            cx,
        );
    })
    .unwrap();
    ui.frame(cx);
    let after = left(&tabs[11], cx);
    assert!(
        after > before,
        "wheel scrolled the strip: {before:?} -> {after:?}"
    );
    assert!(!in_view(&tabs[11], cx), "scrolled partly out of view");

    // A click on the partly hidden active tab scrolls it back into view.
    ui.click(tabs[11].element_id().as_ref(), cx);
    ui.next_frame(cx);
    assert!(in_view(&tabs[11], cx));

    // Closing the active tab: its left neighbor is active and in view.
    ui.press("ctrl-shift-w", cx);
    ui.next_frame(cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[10].id()));
    assert!(in_view(&tabs[10], cx));
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

/// Lead decision 4: the panel never squeezes the main area below 360 px;
/// the chosen width comes back in a wider window.
#[test]
fn details_panel_leaves_the_main_area_its_minimum() {
    use details::fitted_width;
    let sidebar = metrics::WIDTH_SIDEBAR;
    // 960 px window: 960 - 220 - 360 = 380.
    assert_eq!(
        fitted_width(metrics::WIDTH_DETAILS_MAX, px(960.), sidebar),
        metrics::WIDTH_DETAILS
    );
    assert_eq!(
        fitted_width(metrics::WIDTH_DETAILS, px(960.), sidebar),
        metrics::WIDTH_DETAILS
    );
    // Without the sidebar there is room for the widest panel.
    assert_eq!(
        fitted_width(metrics::WIDTH_DETAILS_MAX, px(960.), px(0.)),
        metrics::WIDTH_DETAILS_MAX
    );
    assert_eq!(fitted_width(px(440.), px(1280.), sidebar), px(440.));
    // Never below the panel's own minimum.
    assert_eq!(
        fitted_width(metrics::WIDTH_DETAILS, px(800.), sidebar),
        metrics::WIDTH_DETAILS_MIN
    );
}

/// UI 1.4, 4.6: Exit asks first while sessions are active (connecting,
/// connected, or dropped with automatic attempts ahead); a failed or ended
/// session does not count.
#[gpui_kit::test]
fn exit_asks_while_sessions_are_active(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let record = crate::session_ui::fake::FakeLauncher::install(cx);
    let shell = ui.shell(cx);
    let id = shell.update(cx, |shell, cx| {
        shell.open_session(SessionRequest::new(SessionKind::Local), cx)
    });
    ui.frame(cx);
    assert_eq!(record.panes().len(), 1, "the session started");
    assert_eq!(cx.update(|cx| shell.read(cx).active_sessions(cx)), 1);
    let exit_asks = |cx: &mut TestAppContext| {
        cx.update_window(ui.handle, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.request_exit(window, cx));
        })
        .unwrap();
        ui.frame(cx);
        cx.update_window(ui.handle, |_, window, cx| {
            let open = window.has_active_dialog(cx);
            window.close_all_dialogs(cx);
            open
        })
        .unwrap()
    };
    assert!(exit_asks(cx), "the exit confirmation is open");

    // An ended session is not active.
    let pane = cx.update(|cx| shell.read(cx).pane(id).cloned().unwrap());
    pane.update(cx, |pane, cx| {
        pane.set_phase(
            crate::session_ui::Phase::Ended(crate::session_ui::ExitStatus::Code(1)),
            None,
            cx,
        )
    });
    assert_eq!(cx.update(|cx| shell.read(cx).active_sessions(cx)), 0);
    ui.finish(cx);
}

/// UI 2.1, 2.4, 3: a double click on a card, the host panel's Connect and
/// the search bar's quick connect open SSH session tabs; the tab shows the
/// host name and its OS icon (the server icon until the OS is known).
#[gpui_kit::test]
fn sessions_open_from_cards_the_panel_and_the_search(cx: &mut TestAppContext) {
    use crate::session_ui::fake::Call;
    use crate::sessions::OpenRequest;
    use shellpp_core::service::dto::SessionTarget;

    let ui = Ui::open(cx);
    let record = crate::session_ui::fake::FakeLauncher::install(cx);
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                json!({"label": "web", "address": "192.0.2.10"})
                    .as_object()
                    .cloned()
                    .unwrap(),
                cx,
            )
            .unwrap()
            .record_id
    });
    ui.frame(cx);
    // The OS detected on an earlier connection gives the tab its icon.
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    let vault_id = cx.update(|cx| ui.state.read(cx).vault_id().unwrap());
    core.host_os_detected(
        shellpp_core::service::dto::RecordRef {
            vault_id,
            record_id: host,
        },
        shellpp_core::os_detect::DetectedOs {
            family: shellpp_core::service::dto::OsFamily::Linux,
            distro: None,
            version: None,
        },
        shellpp_core::service::dto::OsSource::Script,
    )
    .unwrap();
    // The core closes (and its temporary profile goes) at the end.
    drop(core);
    ui.frame(cx);
    let card = SharedString::from(format!("host-card-{host}"));
    let shell = ui.shell(cx);
    cx.update_window(ui.handle, move |_, window, cx| {
        window.render_frame(cx);
        window.double_click(card.clone(), cx);
    })
    .unwrap();
    ui.frame(cx);
    let tab = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(tab.kind(), SessionKind::Ssh);
    assert_eq!(tab.title().as_ref(), "web");
    assert_eq!(tab.host(), Some(host));
    assert_eq!(tab.icon(), crate::hosts::layout::os_icon(Some("linux")));
    assert!(matches!(
        &record.calls()[0],
        Call::Launch(OpenRequest::Ssh(SessionTarget::Host { record_id, .. })) if *record_id == host
    ));

    // The host panel's Connect.
    ui.press("ctrl-1", cx);
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_host_panel(Some(host), window, cx)
        });
    })
    .unwrap();
    ui.frame(cx);
    ui.click("host-connect", cx);
    assert_eq!(cx.update(|cx| shell.read(cx).sessions().len()), 2);

    // The search bar's quick connect.
    ui.press("ctrl-1", cx);
    ui.click("search", cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.input("admin@192.0.2.30:2200", cx)
    })
    .unwrap();
    ui.frame(cx);
    ui.click("quick-connect", cx);
    let calls = record.calls();
    let quick = calls
        .iter()
        .filter_map(|call| match call {
            Call::Launch(OpenRequest::Ssh(SessionTarget::Quick {
                address,
                port,
                username,
            })) => Some((address.clone(), *port, username.clone())),
            _ => None,
        })
        .next();
    assert_eq!(
        quick,
        Some((
            "192.0.2.30".to_owned(),
            Some(2200),
            Some("admin".to_owned())
        ))
    );
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[2].icon(), crate::hosts::layout::os_icon(None));
    ui.finish(cx);
}

/// An edit still waiting for the 500 ms autosave is saved when the window
/// is closed (and, through `flush_all_panels`, when the app quits): the
/// panel and its autosave timer go with the window.
#[gpui_kit::test]
fn closing_the_window_saves_a_pending_panel_edit(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                json!({"address": "192.0.2.40"})
                    .as_object()
                    .cloned()
                    .unwrap(),
                cx,
            )
            .unwrap()
            .record_id
    });
    let shell = ui.shell(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_host_panel(Some(host), window, cx)
        });
    })
    .unwrap();
    ui.frame(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input("Closed fast", cx);
    })
    .unwrap();
    ui.frame(cx);
    let label = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            ui.state
                .read(cx)
                .host(host)
                .and_then(|host| host.label.clone())
        })
    };
    assert_eq!(label(cx), None, "still waiting for the autosave");
    let closed = gpui::VisualTestContext::from_window(ui.handle, cx).simulate_close();
    assert!(closed, "no session tabs: the window closes without asking");
    assert_eq!(label(cx).as_deref(), Some("Closed fast"));

    // Quit: every window's panel is saved first.
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input(" again", cx);
    })
    .unwrap();
    ui.frame(cx);
    cx.update(flush_all_panels);
    assert_eq!(label(cx).as_deref(), Some("Closed fast again"));
    ui.finish(cx);
}

/// Ctrl+, opens the Settings window (UI 12.1) on its Security section
/// (UI 13.4); no placeholder toast any more.
#[gpui_kit::test]
fn settings_shortcut_opens_the_settings_window(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    assert!(!ui.visible("settings", cx));
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings", cx));
    assert!(ui.visible("settings-security", cx));
    let shell = ui.shell(cx);
    let toasts = cx.update(|cx| shell.read(cx).toasts().read(cx).items().len());
    assert_eq!(toasts, 0);
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
