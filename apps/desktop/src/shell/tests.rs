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
use crate::theme::metrics;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_on(TestProfile::new().into_unlocked(), cx)
    }

    /// The window on `opened` (a profile opened again: a restart).
    fn open_on(opened: crate::state::OpenedCore, cx: &mut TestAppContext) -> Self {
        // Session tabs have terminals that parse on their own threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
        });
        let state = cx.new(|cx| AppState::new(opened, cx));
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

    /// Replaces the text of the input `id` (click, select all, delete, type).
    fn type_into(&self, id: &str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| {
            window.press("secondary-a", cx);
            window.press("backspace", cx);
            if !text.is_empty() {
                window.input(&text, cx);
            }
        })
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
    assert_eq!(
        main.origin.y,
        metrics::at_100(metrics::HEIGHT_TOP_BAR),
        "top bar height"
    );
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
    assert!(!ui.visible("top-bar-terminal-sidebar", cx));
    // Tabs are 32 px inside the 44 px bar (design-system 6 [decision]).
    for id in ["tab-vaults", "tab-sftp", "tab-new", "vaults-pill"] {
        let tab = ui.bounds(id, cx).expect(id);
        assert_eq!(
            tab.size.height,
            metrics::at_100(metrics::HEIGHT_TAB),
            "{id}"
        );
        // Centered above the bar's 1 px bottom line.
        assert_eq!(
            tab.origin.y,
            (metrics::at_100(metrics::HEIGHT_TOP_BAR)
                - px(1.)
                - metrics::at_100(metrics::HEIGHT_TAB))
                / 2.,
            "{id} centered"
        );
    }
    // The Vaults tab and the vault switcher are one pill; the arrow is a
    // zone of its own inside it, right after the tab.
    let pill = ui.bounds("vaults-pill", cx).unwrap();
    let vaults = ui.bounds("tab-vaults", cx).unwrap();
    let switcher = ui.bounds("vault-switcher", cx).unwrap();
    assert_eq!(pill.origin.x, vaults.origin.x);
    assert_eq!(vaults.right(), switcher.origin.x);
    assert_eq!(switcher.right(), pill.right());
    // 4 px between the pill and the SFTP tab.
    let sftp = ui.bounds("tab-sftp", cx).unwrap();
    assert_eq!(
        sftp.origin.x - pill.right(),
        metrics::at_100(metrics::SPACE_1)
    );
    // Vaults is active: the arrow opens the vault menu, Vaults stays.
    ui.click("vault-switcher", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.press("escape", cx);

    ui.click("tab-sftp", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    // The SFTP tab (stage 4): Local on the left, an empty panel on the right.
    assert!(ui.visible("sftp-screen", cx));
    assert!(ui.visible("sftp-right-empty", cx));
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

/// UI 1.2: six sections, 196 px, 32 px items 2 px apart, no vault header,
/// placeholders.
#[gpui_kit::test]
fn sidebar_sections(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let sidebar = ui.bounds("sidebar", cx).expect("sidebar");
    assert_eq!(sidebar.size.width, metrics::at_100(metrics::WIDTH_SIDEBAR));
    let mut last_bottom = None;
    for section in Section::ALL {
        let item = ui
            .bounds(section.element_id().as_ref(), cx)
            .expect("sidebar item");
        assert_eq!(
            item.size.height,
            metrics::at_100(metrics::HEIGHT_NAV_ITEM),
            "{section:?}"
        );
        // 8 px from the sidebar's edges.
        assert_eq!(
            item.origin.x - sidebar.origin.x,
            metrics::at_100(metrics::SPACE_2)
        );
        assert_eq!(
            sidebar.right() - item.right(),
            metrics::at_100(metrics::SPACE_2) + px(1.)
        );
        if let Some(bottom) = last_bottom {
            assert_eq!(
                item.origin.y - bottom,
                metrics::at_100(metrics::GAP_NAV_ITEM),
                "{section:?}"
            );
        }
        last_bottom = Some(item.bottom());
    }
    // The first item sits at the top of the sidebar: no vault header above it.
    let hosts = ui.bounds("nav-hosts", cx).unwrap();
    assert_eq!(
        hosts.origin.y - sidebar.origin.y,
        metrics::at_100(metrics::SPACE_2)
    );

    // Every section has its screen since stage 6 (D28 placeholders are
    // gone from the sidebar): Logs, the Keychain, Port Forwarding, Snippets.
    ui.click("nav-logs", cx);
    let shell = ui.shell(cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Logs);
    assert!(ui.visible("logs-screen", cx));
    assert!(!ui.visible("section-placeholder", cx));
    assert!(!ui.visible("hosts-screen", cx));
    ui.click("nav-keychain", cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Keychain);
    assert!(ui.visible("keychain-screen", cx));
    assert!(!ui.visible("section-placeholder", cx));
    // Port Forwarding has its screen since stage 5.
    ui.click("nav-port-forwarding", cx);
    assert!(ui.visible("forwarding-screen", cx));
    assert!(!ui.visible("section-placeholder", cx));
    ui.click("nav-snippets", cx);
    assert!(ui.visible("snippets-screen", cx));
    assert!(!ui.visible("section-placeholder", cx));
    ui.click("nav-hosts", cx);
    assert!(ui.visible("hosts-screen", cx));
    ui.finish(cx);
}

/// UI 1.2, 14 (D93): Recently deleted below the six sections, after a
/// separator, opens its screen; it needs no account (local mode).
#[gpui_kit::test]
fn recently_deleted_sits_below_a_separator(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let logs = ui.bounds("nav-logs", cx).unwrap();
    let separator = ui.bounds("sidebar-separator", cx).expect("separator");
    let trash = ui
        .bounds(Section::RecentlyDeleted.element_id().as_ref(), cx)
        .expect("Recently deleted item");
    assert!(separator.top() >= logs.bottom() && trash.top() >= separator.bottom());
    assert_eq!(trash.size.height, metrics::at_100(metrics::HEIGHT_NAV_ITEM));
    assert_eq!(trash.origin.x, logs.origin.x);

    ui.click("nav-recently-deleted", cx);
    let shell = ui.shell(cx);
    assert_eq!(
        cx.update(|cx| shell.read(cx).section()),
        Section::RecentlyDeleted
    );
    assert!(ui.visible("trash-screen", cx));
    assert!(ui.visible("trash-empty-state", cx));
    assert!(!ui.visible("hosts-screen", cx));
    ui.click("nav-hosts", cx);
    assert!(ui.visible("hosts-screen", cx));
    ui.finish(cx);
}

/// D68: another section of the sidebar while the snippet panel holds
/// changes not saved asks first; Stay stays on Snippets, Discard changes
/// goes on.
#[gpui_kit::test]
fn leaving_snippets_with_changes_not_saved_asks_first(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    ui.click("nav-snippets", cx);
    ui.click("snippet-new", cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.click("snippet-script", cx);
        window.input("uptime", cx);
    })
    .unwrap();
    ui.frame(cx);
    ui.click("nav-hosts", cx);
    assert!(ui.visible("snippet-leave-dialog", cx));
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Snippets);
    ui.click("snippet-leave-stay", cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Snippets);
    assert!(ui.visible("snippet-panel", cx));
    ui.click("nav-hosts", cx);
    ui.click("snippet-leave-discard", cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Hosts);
    // Nothing was saved, and the panel of the new snippet is closed.
    ui.click("nav-snippets", cx);
    assert!(!ui.visible("snippet-panel", cx));
    ui.click("nav-hosts", cx);
    assert!(!ui.visible("snippet-leave-dialog", cx));
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Hosts);

    // The Hosts shortcut asks too; Save saves, then goes.
    ui.click("nav-snippets", cx);
    ui.click("snippet-new", cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.click("snippet-script", cx);
        window.input("df -h", cx);
    })
    .unwrap();
    ui.frame(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.dispatch_action(Box::new(super::actions::ShowHosts), cx)
    })
    .unwrap();
    ui.frame(cx);
    assert!(ui.visible("snippet-leave-dialog", cx));
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Snippets);
    ui.click("snippet-leave-save", cx);
    assert_eq!(cx.update(|cx| shell.read(cx).section()), Section::Hosts);
    let saved = ui.state.read_with(cx, |state, _| {
        let core = state.core().cloned().unwrap();
        core.snippets_list(state.vault_id().unwrap()).unwrap()
    });
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].script, "df -h");
    ui.finish(cx);
}

/// The ports of the Serial dialog in these tests (D88): two, never opened
/// here (the tab's session goes through the launcher).
struct TwoPorts;

impl crate::sessions::SerialBackend for TwoPorts {
    fn ports(&self) -> Vec<shellpp_term_local::serial::PortInfo> {
        ["COM10", "COM3"]
            .into_iter()
            .map(|name| shellpp_term_local::serial::PortInfo {
                name: name.into(),
                description: None,
            })
            .collect()
    }

    fn open(
        &self,
        _: &shellpp_term_local::serial::SerialConfig,
    ) -> Result<shellpp_term_local::serial::SerialSession, shellpp_term_local::serial::SerialError>
    {
        Err(shellpp_term_local::serial::SerialError::Missing)
    }
}

fn install_ports(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(serial_dialog::SerialPorts(std::sync::Arc::new(TwoPorts)));
    });
}

/// UI 1.1, 4.6, 12.5: Ctrl+L outside a terminal opens a local terminal
/// tab and Ctrl+Alt+S the Serial dialog, whose Connect opens a serial tab;
/// Alt+Left/Right and Ctrl+N move between tabs; the active tab closes to
/// its neighbor; the last one closes to Vaults.
#[gpui_kit::test]
fn session_tabs_open_and_close(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    assert!(ui.visible("tab-new", cx));

    // Ctrl+L: a local terminal tab, Ctrl+Alt+S: the Serial dialog.
    install_ports(cx);
    ui.press("ctrl-l", cx);
    let local = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(local.kind(), SessionKind::Local);
    assert!(cx.update(|cx| shell.read(cx).pane(local.id()).is_some()));
    // No sidebar on a session tab (UI 4.1); the terminal sidebar button is
    // there (UI 4.2).
    assert!(!ui.visible("sidebar", cx));
    assert!(ui.visible("top-bar-terminal-sidebar", cx));
    assert!(ui.visible("session-pane", cx));
    // Back on Vaults, so that Ctrl+L and Ctrl+Alt+S reach the shell.
    ui.press("ctrl-1", cx);
    ui.press("ctrl-l", cx);
    ui.press("ctrl-alt-s", cx);
    assert!(ui.visible("serial-dialog", cx));
    ui.click("serial-connect", cx);
    assert!(!ui.visible("serial-dialog", cx));
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    assert_eq!(
        tabs.iter().map(SessionTab::kind).collect::<Vec<_>>(),
        [SessionKind::Local, SessionKind::Local, SessionKind::Serial]
    );
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[2].id()));
    assert!(ui.visible("session-pane", cx));

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

/// The Serial dialog (D88): the ports in order with the first chosen, the
/// baud rate from its list and the line settings as segments; Connect
/// opens the port with them in a tab titled after the port, which is not
/// duplicated and not saved for Restore, and the dialog offers them again.
#[gpui_kit::test]
fn the_serial_dialog_opens_the_port_with_its_settings(cx: &mut TestAppContext) {
    use shellpp_term_local::serial::{FlowControl, Parity, SerialConfig, StopBits};
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    install_ports(cx);
    ui.press("ctrl-alt-s", cx);
    // COM3 before COM10, the first one chosen.
    assert!(!ui.visible("serial-port-not-found", cx), "a listed port");
    ui.click("serial-port-choose", cx);
    assert!(ui.visible("serial-port-COM3", cx) && ui.visible("serial-port-COM10", cx));
    let com3 = ui.bounds("serial-port-COM3", cx).expect("COM3");
    let com10 = ui.bounds("serial-port-COM10", cx).expect("COM10");
    assert!(com3.top() < com10.top());
    ui.click("serial-port-COM10", cx);
    assert!(!ui.visible("serial-port-list", cx));
    ui.click("serial-baud", cx);
    ui.click("serial-baud-38400", cx);
    ui.click("serial-parity-even", cx);
    ui.click("serial-stop-2", cx);
    ui.click("serial-flow-hardware", cx);
    ui.click("serial-connect", cx);

    let mut expected = SerialConfig::new("COM10");
    expected.baud = 38_400;
    expected.parity = Parity::Even;
    expected.stop_bits = StopBits::Two;
    expected.flow = FlowControl::Hardware;
    let tab = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(tab.kind(), SessionKind::Serial);
    assert_eq!(tab.title().as_ref(), "COM10");
    let (target, facts, snapshot) = cx.update(|cx| {
        let shell = shell.read(cx);
        let group = shell.group(tab.id()).expect("panes").read(cx);
        (
            group.active_target(cx),
            shell.tab_facts(tab.id(), cx),
            group.snapshot(cx),
        )
    });
    assert_eq!(target, Some(PaneTarget::Serial(expected.clone())));
    // A port opens once: no Duplicate; a split still works. Never saved.
    assert!(!facts.can_duplicate && facts.has_target);
    assert!(snapshot.is_none());

    // The dialog again (from Vaults, where the shell has the keys): the
    // last settings.
    ui.press("ctrl-1", cx);
    ui.press("ctrl-alt-s", cx);
    ui.click("serial-connect", cx);
    let tabs = cx.update(|cx| shell.read(cx).sessions().tabs().to_vec());
    assert_eq!(tabs.len(), 2);
    let target = cx.update(|cx| {
        shell
            .read(cx)
            .group(tabs[1].id())
            .expect("panes")
            .read(cx)
            .active_target(cx)
    });
    assert_eq!(target, Some(PaneTarget::Serial(expected)));
    ui.finish(cx);
}

/// The serial target of the last session tab.
fn last_serial_target(ui: &Ui, cx: &mut TestAppContext) -> Option<PaneTarget> {
    let shell = ui.shell(cx);
    cx.update(|cx| {
        let shell = shell.read(cx);
        let tab = shell.sessions().tabs().last()?.clone();
        shell.group(tab.id())?.read(cx).active_target(cx)
    })
}

/// UI 1.4, D96: Ctrl+= / Ctrl+- / Ctrl+0 outside a terminal and View >
/// Zoom In / Zoom Out / Actual Size scale the whole interface by the steps
/// (80 to 200 %), keep the zoom for the next launch and leave the
/// terminal's font as it is; in a terminal the keys go to the program.
#[gpui_kit::test]
fn the_interface_zoom_steps_is_kept_and_leaves_the_terminal_font(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    let zoom = |cx: &mut TestAppContext| cx.update(|cx| metrics::zoom(cx));
    let stored = |ui: &Ui, cx: &mut TestAppContext| {
        ui.state.read_with(cx, |state, _| {
            state.core().unwrap().settings_get().unwrap()["appearance.ui_zoom"].clone()
        })
    };
    let top_bar =
        |ui: &Ui, cx: &mut TestAppContext| ui.bounds("main-area", cx).expect("main area").origin.y;
    assert_eq!(zoom(cx), 1.);
    assert_eq!(top_bar(&ui, cx), px(44.));
    ui.press("ctrl-=", cx);
    assert_eq!(zoom(cx), 1.1);
    assert_eq!(stored(&ui, cx), json!(1.1));
    assert!((top_bar(&ui, cx) - px(44. * 1.1)).abs() < px(0.5));
    ui.press("ctrl-=", cx);
    assert_eq!(zoom(cx), 1.25);
    ui.press("ctrl--", cx);
    ui.press("ctrl--", cx);
    ui.press("ctrl--", cx);
    ui.press("ctrl--", cx);
    ui.press("ctrl--", cx);
    assert_eq!(zoom(cx), 0.8, "the smallest step stays");
    assert!((top_bar(&ui, cx) - px(44. * 0.8)).abs() < px(0.5));
    ui.press("ctrl-0", cx);
    assert_eq!(zoom(cx), 1.);
    assert_eq!(stored(&ui, cx), json!(1.0));
    assert_eq!(top_bar(&ui, cx), px(44.));

    // A terminal: the keys go to the program, and the zoom of the menu
    // leaves its font (the grid keeps its columns).
    ui.press("ctrl-l", cx);
    let tab = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    let columns = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let pane = shell.read(cx).pane(tab.id()).expect("pane").read(cx);
            pane.terminal()
                .expect("terminal")
                .read(cx)
                .handle()
                .size()
                .cols
        })
    };
    ui.next_frame(cx);
    let before = columns(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        let pane = shell.read(cx).pane(tab.id()).expect("pane").clone();
        let terminal = pane.read(cx).terminal().expect("terminal").clone();
        terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
    })
    .unwrap();
    ui.frame(cx);
    for key in ["ctrl-=", "ctrl--", "ctrl-0"] {
        ui.press(key, cx);
        assert_eq!(zoom(cx), 1., "{key} went to the program");
    }
    // What View > Zoom In runs (twice).
    shell.update(cx, |shell, cx| {
        shell.zoom(Some(true), cx);
        shell.zoom(Some(true), cx);
    });
    ui.next_frame(cx);
    assert_eq!(zoom(cx), 1.25);
    assert_eq!(columns(cx), before, "the terminal's font is its own");
    // The kit's own radii follow (they are pixels in the kit).
    assert_eq!(
        cx.update(|cx| gpui_kit::component::Theme::global(cx).radius),
        px(8.)
    );

    // The next launch starts at the kept zoom.
    cx.update(|cx| crate::theme::zoom::set(1., cx));
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    cx.update(|cx| crate::theme::zoom::apply_stored(&core, cx));
    assert_eq!(zoom(cx), 1.25);

    // In Settings (a dialog of the kit, outside the shell) the keys work.
    ui.click("tab-vaults", cx);
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings", cx));
    ui.press("ctrl--", cx);
    assert_eq!(zoom(cx), 1.1);
    ui.press("escape", cx);

    // A zoom the settings get from elsewhere (an import) applies at once.
    core.settings_set(shellpp_core::service::dto::SettingsSetInput {
        values: [("appearance.ui_zoom".to_owned(), json!(1.5))]
            .into_iter()
            .collect(),
        reset: Vec::new(),
    })
    .unwrap();
    ui.frame(cx);
    assert_eq!(zoom(cx), 1.5);
    cx.update(|cx| crate::theme::zoom::set(1., cx));
    ui.finish(cx);
}

/// D95: a port name typed by hand opens like a listed one, marked "Not
/// found now" when the system does not list it; an empty field leaves
/// Connect disabled. The settings of the last Connect outlive a restart,
/// and a saved port that is not listed stays in the field, not replaced by
/// the first port found.
#[gpui_kit::test]
fn the_serial_dialog_takes_a_typed_port_and_keeps_it_after_a_restart(cx: &mut TestAppContext) {
    use shellpp_term_local::serial::{Parity, SerialConfig};
    let mut profile = TestProfile::new();
    let ui = Ui::open_on(profile.open_unlocked(), cx);
    install_ports(cx);
    ui.press("ctrl-alt-s", cx);
    ui.type_into("serial-port", "", cx);
    ui.click("serial-connect", cx);
    assert!(ui.visible("serial-dialog", cx), "no port, no Connect");
    // A name longer than a port name can be: said, and no Connect.
    ui.type_into("serial-port", &"A".repeat(257), cx);
    assert!(ui.visible("serial-port-too-long", cx));
    ui.click("serial-connect", cx);
    assert!(ui.visible("serial-dialog", cx), "too long, no Connect");
    // Windows names are not case-sensitive: `com3` is the listed COM3.
    if cfg!(windows) {
        ui.type_into("serial-port", "com3", cx);
        assert!(!ui.visible("serial-port-not-found", cx));
    }
    ui.type_into("serial-port", r"  \\.\COM12 ", cx);
    assert!(ui.visible("serial-port-not-found", cx));
    ui.click("serial-parity-odd", cx);
    ui.click("serial-baud", cx);
    ui.click("serial-baud-19200", cx);
    ui.click("serial-connect", cx);
    assert!(!ui.visible("serial-dialog", cx));
    let mut expected = SerialConfig::new(r"\\.\COM12");
    expected.baud = 19_200;
    expected.parity = Parity::Odd;
    assert_eq!(
        last_serial_target(&ui, cx),
        Some(PaneTarget::Serial(expected.clone()))
    );
    ui.finish(cx);

    // Restart: the same profile, a new app state.
    let ui = Ui::open_on(profile.open_unlocked(), cx);
    install_ports(cx);
    ui.press("ctrl-alt-s", cx);
    assert!(
        ui.visible("serial-port-not-found", cx),
        "still not found now"
    );
    // Enter in the port field is Connect.
    ui.click("serial-port", cx);
    ui.press("enter", cx);
    assert!(!ui.visible("serial-dialog", cx));
    assert_eq!(
        last_serial_target(&ui, cx),
        Some(PaneTarget::Serial(expected))
    );
    // Chosen from the list, the mark goes.
    ui.press("ctrl-1", cx);
    ui.press("ctrl-alt-s", cx);
    ui.click("serial-port-choose", cx);
    ui.click("serial-port-COM3", cx);
    assert!(!ui.visible("serial-port-not-found", cx));
    ui.click("serial-connect", cx);
    let target = last_serial_target(&ui, cx);
    assert!(
        matches!(&target, Some(PaneTarget::Serial(config)) if config.port == "COM3" && config.baud == 19_200),
        "{target:?}"
    );
    ui.finish(cx);
    drop(profile);
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
        width >= metrics::at_100(title_bar::SESSION_TAB_MIN_WIDTH) - px(0.5),
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
    assert_eq!(
        main.bottom() - toast.bottom(),
        metrics::at_100(metrics::SPACE_4)
    );

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
    assert_eq!(panel.size.width, metrics::at_100(metrics::WIDTH_DETAILS));
    let main = ui.bounds("main-area", cx).unwrap();
    assert_eq!(
        main.size.width,
        full.size.width - metrics::at_100(metrics::WIDTH_DETAILS)
    );
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
        metrics::at_100(metrics::WIDTH_DETAILS_MAX)
    );
    shell.update(cx, |shell, cx| shell.set_details_width(px(10.), cx));
    assert_eq!(
        cx.update(|cx| shell.read(cx).details_width()),
        metrics::at_100(metrics::WIDTH_DETAILS_MIN)
    );
    ui.finish(cx);
}

/// Lead decision 4: the panel never squeezes the main area below 360 px;
/// the chosen width comes back in a wider window.
#[test]
fn details_panel_leaves_the_main_area_its_minimum() {
    use details::fitted_width;
    let sidebar = metrics::at_100(metrics::WIDTH_SIDEBAR);
    // 960 px window: 960 - 196 - 360 = 404.
    assert_eq!(
        fitted_width(
            metrics::at_100(metrics::WIDTH_DETAILS_MAX),
            px(960.),
            sidebar
        ),
        px(404.)
    );
    assert_eq!(
        fitted_width(metrics::at_100(metrics::WIDTH_DETAILS), px(960.), sidebar),
        metrics::at_100(metrics::WIDTH_DETAILS)
    );
    // Without the sidebar there is room for the widest panel.
    assert_eq!(
        fitted_width(
            metrics::at_100(metrics::WIDTH_DETAILS_MAX),
            px(960.),
            px(0.)
        ),
        metrics::at_100(metrics::WIDTH_DETAILS_MAX)
    );
    assert_eq!(fitted_width(px(440.), px(1280.), sidebar), px(440.));
    // Never below the panel's own minimum.
    assert_eq!(
        fitted_width(metrics::at_100(metrics::WIDTH_DETAILS), px(800.), sidebar),
        metrics::at_100(metrics::WIDTH_DETAILS_MIN)
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

/// UI 1.1, D100: the update button shows only while a check of this run
/// found an update, and opens its release page.
#[gpui_kit::test]
fn the_update_button_shows_an_update_found(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    assert!(!ui.visible("top-bar-update", cx), "no update found yet");
    cx.update(|cx| {
        cx.set_global(crate::updates::Found(Some((
            "9.9.9".to_owned(),
            "https://example.com/releases/v9.9.9".to_owned(),
        ))));
    });
    ui.frame(cx);
    assert!(ui.visible("top-bar-update", cx));
    // Left of the notification center.
    let update = ui.bounds("top-bar-update", cx).expect("button");
    let bell = ui.bounds("top-bar-notifications", cx).expect("bell");
    assert!(update.right() <= bell.left());
    ui.click("top-bar-update", cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/releases/v9.9.9")
    );
    // A later check that finds nothing newer hides it.
    cx.update(|cx| cx.set_global(crate::updates::Found(None)));
    ui.frame(cx);
    assert!(!ui.visible("top-bar-update", cx));
    ui.finish(cx);
}
