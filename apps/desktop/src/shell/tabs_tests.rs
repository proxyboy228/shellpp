//! Headless tests of stage 2b in the window shell (GPUI `TestAppContext`,
//! a temporary profile per test, the fake session layer: no network, no
//! PTY, no keys): the tab icons and state dot (UI 1.1), the tab menu and
//! Rename (UI 4.4), Ctrl+Shift+W with splits (UI 4.7), the new tab page,
//! "+", Ctrl+T, Ctrl+K (UI 6, 12.5), several windows and Alt on connect
//! actions (UI 1.4, 2.4), the terminal sidebar (UI 4.2) and Restore (UI 6;
//! lead decision L5). Elements are found by element id (UI 0).

use gpui::{
    AnyWindowHandle, Bounds, Entity, Focusable as _, InputEvent as _, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, SharedString, TestAppContext, VisualTestContext,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::app::{self, RootView};
use crate::hosts::QuickTarget;
use crate::panes::{PaneId, PaneTarget, SplitPath};
use crate::session_ui::fake::{Call, FakeLauncher, Record};
use crate::session_ui::{ConnectStep, ExitStatus, Failure};
use crate::sessions::OpenRequest;
use crate::state::AppState;
use crate::testing::TestProfile;
use shellpp_core::service::dto::SessionTarget;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
    record: Record,
    password: secrecy::SecretString,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        // Terminals parse on their own threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
        });
        let record = FakeLauncher::install(cx);
        let profile = TestProfile::new();
        let password = profile.password().clone();
        let state = cx.new(|cx| AppState::new(profile.into_unlocked(), cx));
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
            record,
            password,
        };
        ui.frame(cx);
        ui
    }

    fn shell(&self, cx: &mut TestAppContext) -> Entity<Shell> {
        cx.update(|cx| self.root.read(cx).shell().cloned().expect("shell"))
    }

    /// The shell of window `handle`.
    fn shell_of(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Entity<Shell> {
        cx.update(|cx| {
            all_windows(cx)
                .into_iter()
                .find(|(window, _)| window.window_id() == handle.window_id())
                .map(|(_, shell)| shell)
                .expect("shell")
        })
    }

    fn frame(&self, cx: &mut TestAppContext) {
        Self::frame_of(self.handle, cx);
    }

    fn frame_of(handle: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn click(&self, id: impl Into<SharedString>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.frame(cx);
    }

    fn right_click(&self, id: impl Into<SharedString>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.right_click(id.clone(), cx);
        })
        .unwrap();
        self.frame(cx);
    }

    /// `count` clicks on element `id` of window `handle` with Alt held
    /// (UI 2.4).
    fn alt_click(handle: AnyWindowHandle, id: &str, count: usize, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        let modifiers = Modifiers {
            alt: true,
            ..Default::default()
        };
        cx.update_window(handle, move |_, window, cx| {
            window.render_frame(cx);
            let position = window.find(id.clone()).bounds().center();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            for click_count in 1..=count {
                window.dispatch_event(
                    MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers,
                        click_count,
                    }
                    .to_platform_input(),
                    cx,
                );
            }
        })
        .unwrap();
        Self::frame_of(handle, cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.frame(cx);
    }

    fn input(&self, text: &str, cx: &mut TestAppContext) {
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.input(&text, cx))
            .unwrap();
        self.frame(cx);
    }

    fn visible(&self, id: impl Into<SharedString>, cx: &mut TestAppContext) -> bool {
        Self::visible_in(self.handle, id, cx)
    }

    fn visible_in(
        handle: AnyWindowHandle,
        id: impl Into<SharedString>,
        cx: &mut TestAppContext,
    ) -> bool {
        let id = id.into();
        cx.update_window(handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .is_some_and(|element| element.visible())
        })
        .unwrap()
    }

    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|element| element.focused()) == Some(true)
        })
        .unwrap()
    }

    fn tabs(&self, cx: &mut TestAppContext) -> Vec<SessionTab> {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).sessions().tabs().to_vec())
    }

    fn top_tab(&self, cx: &mut TestAppContext) -> TopTab {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).top_tab())
    }

    fn save_host(&self, data: serde_json::Value, cx: &mut TestAppContext) -> Uuid {
        let id = self.state.update(cx, |state, cx| {
            state
                .save_host(None, data.as_object().cloned().unwrap(), cx)
                .unwrap()
                .record_id
        });
        self.frame(cx);
        id
    }

    fn open_tab(&self, request: SessionRequest, cx: &mut TestAppContext) -> SessionId {
        let shell = self.shell(cx);
        let id = shell.update(cx, |shell, cx| shell.open_session(request, cx));
        self.frame(cx);
        id
    }

    /// The session of tab `id`'s active pane.
    fn pane(&self, id: SessionId, cx: &mut TestAppContext) -> Entity<SessionPane> {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).pane(id).cloned().expect("pane"))
    }

    fn group(&self, id: SessionId, cx: &mut TestAppContext) -> Entity<PaneGroup> {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).group(id).cloned().expect("group"))
    }

    fn set_phase(&self, pane: &Entity<SessionPane>, phase: Phase, cx: &mut TestAppContext) {
        pane.update(cx, |pane, cx| pane.set_phase(phase, None, cx));
        self.frame(cx);
    }

    fn launches(&self) -> Vec<OpenRequest> {
        self.record
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Launch(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    fn windows(cx: &mut TestAppContext) -> Vec<AnyWindowHandle> {
        cx.update(|cx| cx.windows())
    }

    fn finish(self, cx: &mut TestAppContext) {
        for handle in Self::windows(cx) {
            let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        }
        cx.run_until_parked();
        cx.update(release_all_sessions);
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn web() -> serde_json::Value {
    json!({"label": "web", "address": "192.0.2.10"})
}

/// Owner's stage 2b steps 0.1 and 0.2 (UI 1.1): the left icon is the OS
/// icon (the server icon until the OS is known, never the terminal glyph),
/// the type icon is on the right; the 8 px dot shows connecting, connected
/// and drop, and goes with the session.
#[gpui_kit::test]
fn tab_icons_and_the_state_dot(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let tab = ui.tabs(cx)[0].clone();
    assert_eq!(tab.icon(), Lucide::Server, "OS not known yet");
    assert_eq!(tab.kind().type_icon(), Some(Lucide::SquareTerminal));
    let shell = ui.shell(cx);
    let status = |cx: &mut TestAppContext| cx.update(|cx| shell.read(cx).tab_status(id, cx));

    // Connecting: the dot right after the title.
    assert_eq!(status(cx), Some(PaneStatus::Connecting));
    let dot = tab.dot_element_id();
    assert!(ui.visible(dot.clone(), cx));
    let title_bounds = cx
        .update_window(ui.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(dot.clone()).bounds()
        })
        .unwrap();
    assert_eq!(title_bounds.size.width, metrics::SIZE_STATUS_DOT);

    let pane = ui.pane(id, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    assert_eq!(status(cx), Some(PaneStatus::Connected));
    // A drop: the dot turns to danger and the terminal is dimmed without
    // a cursor (the terminal's own tests check the drawing).
    ui.set_phase(&pane, Phase::Dropped(Failure::Other), cx);
    assert_eq!(status(cx), Some(PaneStatus::Dropped));
    assert!(ui.visible(dot.clone(), cx));
    let terminal = cx.update(|cx| pane.read(cx).terminal().cloned().expect("terminal"));
    assert!(cx.update(|cx| terminal.read(cx).is_dimmed()));
    ui.set_phase(&pane, Phase::Reconnecting(ConnectStep::Resolving), cx);
    assert_eq!(status(cx), Some(PaneStatus::Connecting));
    ui.set_phase(&pane, Phase::Running, cx);
    assert!(!cx.update(|cx| terminal.read(cx).is_dimmed()));
    // A session ended with a non-zero code: inactive.
    ui.set_phase(&pane, Phase::Ended(ExitStatus::Code(1)), cx);
    assert_eq!(status(cx), Some(PaneStatus::Ended));
    assert!(ui.visible(dot, cx));

    // The OS detected later: the penguin, never the terminal glyph.
    pane.update(cx, |pane, cx| {
        pane.set_info(
            "web".into(),
            crate::hosts::layout::os_icon(Some("linux")),
            cx,
        )
    });
    ui.frame(cx);
    let tab = ui.tabs(cx)[0].clone();
    assert_eq!(tab.icon(), crate::app::assets::LINUX_ICON);
    assert_ne!(tab.icon(), Lucide::SquareTerminal);

    // A local terminal: the monitor on the left, the terminal on the right.
    let local = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let tab = ui
        .tabs(cx)
        .into_iter()
        .find(|tab| tab.id() == local)
        .unwrap();
    assert_eq!(tab.icon(), Lucide::Monitor);
    assert_eq!(tab.kind().type_icon(), Some(Lucide::SquareTerminal));
    ui.finish(cx);
}

/// UI 1.1, 6, 12.5: "+" and Ctrl+T (outside a terminal) open a "New Tab"
/// tab with the new tab page and its search focused; in a terminal Ctrl+T
/// goes to the shell and Ctrl+Shift+T opens the page.
#[gpui_kit::test]
fn plus_and_ctrl_t_open_the_new_tab_page(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.click("tab-new", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].kind(), SessionKind::NewTab);
    assert_eq!(tabs[0].title().as_ref(), i18n::sessions::NEW_TAB);
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[0].id()));
    assert!(ui.visible("new-tab-page", cx));
    assert!(ui.focused("new-tab-search", cx), "the search is focused");
    // No dot without a session.
    assert!(!ui.visible(tabs[0].dot_element_id(), cx));

    ui.press("ctrl-1", cx);
    ui.press("ctrl-t", cx);
    assert_eq!(ui.tabs(cx).len(), 2);

    // In a running terminal: Ctrl+T is the shell's, Ctrl+Shift+T is ours.
    let local = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let pane = ui.pane(local, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    let terminal = cx.update(|cx| pane.read(cx).terminal().cloned().unwrap());
    let in_terminal = cx
        .update_window(ui.handle, |_, window, cx| {
            terminal.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(in_terminal, "the terminal has the focus");
    ui.press("ctrl-t", cx);
    assert_eq!(ui.tabs(cx).len(), 3, "Ctrl+T went to the shell");
    ui.press("ctrl-shift-t", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 4);
    assert_eq!(tabs[3].kind(), SessionKind::NewTab);
    ui.finish(cx);
}

/// UI 6: choosing a host on the new tab page opens the session in the same
/// tab (same place, same id).
#[gpui_kit::test]
fn the_new_tab_page_opens_the_session_in_its_tab(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let other = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    ui.click("tab-new", cx);
    let page = ui.tabs(cx)[1].id();
    // The page offers the other tab, never itself.
    ui.input("web", cx);
    ui.press("enter", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].id(), other);
    assert_eq!(tabs[1].id(), page, "the same tab");
    assert_eq!(tabs[1].kind(), SessionKind::Ssh);
    assert_eq!(tabs[1].title().as_ref(), "web");
    assert_eq!(tabs[1].host(), Some(host));
    assert_eq!(ui.top_tab(cx), TopTab::Session(page));
    assert!(!ui.visible("new-tab-page", cx));
    assert!(matches!(
        ui.launches().last(),
        Some(OpenRequest::Ssh(SessionTarget::Host { record_id, .. })) if *record_id == host
    ));
    ui.finish(cx);
}

/// UI 6, 12.5: Ctrl+K (outside a terminal) and Ctrl+Shift+K (anywhere)
/// open the search over the window; arrows and Enter choose, an open tab
/// is activated, a host opens in a new tab; Esc closes it.
#[gpui_kit::test]
fn ctrl_k_searches_tabs_and_hosts(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let local = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    ui.press("ctrl-1", cx);

    ui.press("ctrl-k", cx);
    assert!(ui.visible("search-overlay", cx));
    // A host: Enter opens it in a new tab and closes the overlay.
    ui.input("web", cx);
    ui.press("enter", cx);
    assert!(!ui.visible("search-overlay", cx));
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[1].host(), Some(host));
    assert_eq!(ui.top_tab(cx), TopTab::Session(tabs[1].id()));

    // Esc closes it.
    ui.press("ctrl-1", cx);
    ui.press("ctrl-k", cx);
    assert!(ui.visible("search-overlay", cx));
    ui.press("escape", cx);
    assert!(!ui.visible("search-overlay", cx));

    // An open tab: Down, Up and Enter bring it forward.
    ui.press("ctrl-k", cx);
    ui.input("local", cx);
    ui.press("down", cx);
    ui.press("up", cx);
    ui.press("enter", cx);
    assert!(!ui.visible("search-overlay", cx));
    assert_eq!(ui.top_tab(cx), TopTab::Session(local));
    assert_eq!(ui.tabs(cx).len(), 2, "no new tab");

    // In a running terminal: Ctrl+K is the shell's, Ctrl+Shift+K is ours.
    let pane = ui.pane(local, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    ui.press("ctrl-k", cx);
    assert!(!ui.visible("search-overlay", cx));
    ui.press("ctrl-shift-k", cx);
    assert!(ui.visible("search-overlay", cx));
    ui.press("escape", cx);
    ui.finish(cx);
}

/// UI 4.4: Duplicate (next to the tab, same target), Split horizontally
/// and vertically (Broadcast input appears, disabled), Close.
#[gpui_kit::test]
fn the_tab_menu_duplicates_splits_and_closes(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let first = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let last = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let tab = ui.tabs(cx)[0].clone();

    ui.right_click(tab.element_id(), cx);
    for id in [
        "tab-menu-duplicate",
        "tab-menu-duplicate-window",
        "tab-menu-rename",
        "tab-menu-split-horizontal",
        "tab-menu-split-vertical",
        "tab-menu-close",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert!(
        ui.visible("tab-menu-broadcast", cx),
        "another tab is open (D86)"
    );
    ui.click("tab-menu-duplicate", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[0].id(), first);
    assert_eq!(tabs[1].title().as_ref(), "web", "right after its tab");
    assert_eq!(tabs[2].id(), last);
    let launches = ui.launches();
    assert_eq!(launches.len(), 3);
    assert_eq!(launches[2], launches[0], "the same target");

    // Split horizontally: a new pane below, with the host picker.
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-split-horizontal", cx);
    let group = ui.group(first, cx);
    let layout = cx.update(|cx| group.read(cx).layout().clone());
    assert_eq!(layout.leaf_count(), 2);
    assert_eq!(layout.dividers()[0].direction, Direction::Column);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    // The new pane chooses its host first: the splits are disabled while
    // it shows the picker (UI 4.4).
    let (below, target) = cx.update(|cx| {
        let group = group.read(cx);
        let first_pane = group.pane_ids()[0];
        (group.active_pane(), group.target(first_pane, cx).unwrap())
    });
    group.update(cx, |group, cx| group.open_in(below, &target, cx));
    ui.frame(cx);
    // Split vertically: a new pane to the right of the active one.
    ui.right_click(tab.element_id(), cx);
    assert!(ui.visible("tab-menu-broadcast", cx), "a tab with splits");
    ui.click("tab-menu-split-vertical", cx);
    let layout = cx.update(|cx| group.read(cx).layout().clone());
    assert_eq!(layout.leaf_count(), 3);
    assert!(
        layout
            .dividers()
            .iter()
            .any(|divider| divider.direction == Direction::Row)
    );
    // Broadcast input (UI 4.4): on, the receiving panes of the running
    // sessions are marked while one of them is active (the new pane still
    // chooses a host: typing there is not mirrored); the item then stops it.
    let sessions: Vec<Entity<SessionPane>> =
        cx.update(|cx| group.read(cx).sessions().into_iter().cloned().collect());
    for session in &sessions {
        session.update(cx, |session, cx| {
            session.set_phase(Phase::Running, None, cx)
        });
    }
    // Other tabs are open: the item lists them, this tab ticked (D86).
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-broadcast", cx);
    assert!(ui.visible("broadcast-dialog", cx));
    ui.click("broadcast-start", cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 3);
    assert!(cx.update(|cx| group.read(cx).broadcasting()));
    let picker = cx.update(|cx| group.read(cx).active_pane());
    assert!(cx.update(|cx| group.read(cx).session(picker).is_none()));
    for pane in cx.update(|cx| group.read(cx).pane_ids()) {
        assert!(!ui.visible(format!("pane-{pane}-broadcast"), cx));
    }
    let members: Vec<PaneId> = cx.update(|cx| {
        let group = group.read(cx);
        group
            .pane_ids()
            .into_iter()
            .filter(|pane| group.session(*pane).is_some())
            .collect()
    });
    assert_eq!(members.len(), 2);
    group.update(cx, |group, cx| group.activate(members[0], cx));
    ui.frame(cx);
    for pane in cx.update(|cx| group.read(cx).pane_ids()) {
        assert_eq!(
            ui.visible(format!("pane-{pane}-broadcast"), cx),
            pane == members[1]
        );
    }
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-broadcast", cx);
    assert!(!cx.update(|cx| group.read(cx).broadcasting()));
    for pane in cx.update(|cx| group.read(cx).pane_ids()) {
        assert!(!ui.visible(format!("pane-{pane}-broadcast"), cx));
    }
    // Ctrl+Shift+B turns it on and off too; with one connected pane a
    // toast says it needs two.
    ui.press("ctrl-shift-b", cx);
    assert!(cx.update(|cx| group.read(cx).broadcasting()));
    ui.press("ctrl-shift-b", cx);
    assert!(!cx.update(|cx| group.read(cx).broadcasting()));
    sessions[1].update(cx, |session, cx| {
        session.set_phase(Phase::Dropped(Failure::ConnectionLost), None, cx)
    });
    let shell = ui.shell(cx);
    let toasts =
        |cx: &mut TestAppContext| cx.update(|cx| shell.read(cx).toasts().read(cx).items().len());
    let before = toasts(cx);
    ui.press("ctrl-shift-b", cx);
    assert!(!cx.update(|cx| group.read(cx).broadcasting()));
    assert_eq!(toasts(cx), before + 1);
    sessions[1].update(cx, |session, cx| {
        session.set_phase(Phase::Running, None, cx)
    });

    // Close: the whole tab with its panes.
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-close", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().all(|tab| tab.id() != first));
    ui.finish(cx);
}

/// D86: Broadcast input over ticked tabs. The tab menu lists the open tabs;
/// what is typed in one goes to the connected panes of the ticked tabs, each
/// marked in the tab strip, and not to the others; a tab that closes leaves,
/// and fewer than two connected panes in all end it.
#[gpui_kit::test]
fn broadcast_input_reaches_the_ticked_tabs(cx: &mut TestAppContext) {
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<u8>>);
    impl crate::terminal::TerminalSink for Sink {
        fn send(&self, bytes: Vec<u8>) {
            self.0.lock().unwrap().extend_from_slice(&bytes);
        }
        fn resize(&self, _: crate::terminal::TerminalSize) {}
    }

    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let a = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let b = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let c = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let mut terminals = Vec::new();
    for id in [a, b, c] {
        let group = ui.group(id, cx);
        let session = cx.update(|cx| group.read(cx).sessions()[0].clone());
        session.update(cx, |session, cx| {
            session.set_phase(Phase::Running, None, cx)
        });
        let terminal = cx.update(|cx| session.read(cx).terminal().cloned().unwrap());
        let sink = std::sync::Arc::new(Sink::default());
        cx.update(|cx| terminal.read(cx).handle().set_sink(Some(sink.clone())));
        terminals.push((terminal, sink));
    }
    ui.frame(cx);
    let tab_of = |id: SessionId, cx: &mut TestAppContext| {
        ui.tabs(cx)
            .into_iter()
            .find(|tab| tab.id() == id)
            .unwrap()
            .element_id()
    };

    // Tab A's menu: the list with A ticked; B and C ticked too.
    let a_tab = tab_of(a, cx);
    ui.right_click(a_tab.clone(), cx);
    ui.click("tab-menu-broadcast", cx);
    assert!(ui.visible("broadcast-dialog", cx));
    ui.click(format!("broadcast-tab-{b}"), cx);
    ui.click(format!("broadcast-tab-{c}"), cx);
    ui.click("broadcast-start", cx);
    let broadcasting = |id: SessionId, cx: &mut TestAppContext| {
        let group = ui.group(id, cx);
        cx.update(|cx| group.read(cx).broadcasting())
    };
    assert!(broadcasting(a, cx) && broadcasting(b, cx) && broadcasting(c, cx));
    assert!(ui.visible(format!("{a_tab}-broadcast"), cx));
    assert!(ui.visible(format!("{}-broadcast", tab_of(b, cx)), cx));

    // Typed in B: A and C get it.
    terminals[1]
        .0
        .update(cx, |terminal, cx| terminal.type_for_test("date\r", cx));
    assert_eq!(terminals[0].1.0.lock().unwrap().as_slice(), b"date\r");
    assert_eq!(terminals[2].1.0.lock().unwrap().as_slice(), b"date\r");

    // C's session drops: C leaves (no tower), A and B go on.
    let c_session = {
        let group = ui.group(c, cx);
        cx.update(|cx| group.read(cx).sessions()[0].clone())
    };
    c_session.update(cx, |session, cx| {
        session.set_phase(Phase::Dropped(Failure::ConnectionLost), None, cx)
    });
    ui.frame(cx);
    assert!(!broadcasting(c, cx));
    assert!(!ui.visible(format!("{}-broadcast", tab_of(c, cx)), cx));
    assert!(broadcasting(a, cx) && broadcasting(b, cx));

    // Typed in A: B gets it, C does not.
    terminals[0]
        .0
        .update(cx, |terminal, cx| terminal.type_for_test("uptime\r", cx));
    assert_eq!(
        terminals[1].1.0.lock().unwrap().as_slice(),
        b"date\ruptime\r"
    );
    assert_eq!(terminals[2].1.0.lock().unwrap().as_slice(), b"date\r");

    // B closes: A alone with one pane ends it.
    let shell = ui.shell(cx);
    shell.update(cx, |shell, cx| shell.close_session(b, cx));
    ui.frame(cx);
    assert!(!broadcasting(a, cx));
    assert!(!ui.visible(format!("{a_tab}-broadcast"), cx));
    ui.finish(cx);
}

/// UI 4.4: Rename edits the title in place: Enter saves, Esc cancels, empty
/// brings back the host's name; the host is not renamed.
#[gpui_kit::test]
fn rename_edits_the_tab_title_in_place(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let tab = ui.tabs(cx)[0].clone();
    let shell = ui.shell(cx);
    let title = |cx: &mut TestAppContext| ui.tabs(cx)[0].title().to_string();

    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    assert!(ui.visible(tab.rename_element_id(), cx));
    assert!(cx.update(|cx| shell.read(cx).renaming().is_some()));
    // The title is selected: typing replaces it.
    ui.input("prod web", cx);
    ui.press("enter", cx);
    assert!(cx.update(|cx| shell.read(cx).renaming().is_none()));
    assert!(!ui.visible(tab.rename_element_id(), cx));
    assert_eq!(title(cx), "prod web");
    assert_eq!(
        cx.update(|cx| ui.state.read(cx).host(host).unwrap().title().to_owned()),
        "web",
        "the host keeps its name"
    );

    // Esc cancels.
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    ui.input("other", cx);
    ui.press("escape", cx);
    assert!(cx.update(|cx| shell.read(cx).renaming().is_none()));
    assert_eq!(title(cx), "prod web");

    // Empty: the host's name again.
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    ui.press("backspace", cx);
    ui.press("enter", cx);
    assert_eq!(title(cx), "web");
    assert_eq!(ui.tabs(cx)[0].custom_title(), None);
    assert_eq!(ui.top_tab(cx), TopTab::Session(id));
    ui.finish(cx);
}

/// UI 4.4: Rename from the menu of a tab that is not active stays open
/// (the pane of the tab does not take the focus away), also when the pane
/// asks for the focus meanwhile; Enter on the unchanged title keeps the
/// tab following its host.
#[gpui_kit::test]
fn rename_stays_open_until_the_user_ends_it(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let first = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let second = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(second));
    let tab = ui.tabs(cx)[0].clone();
    let shell = ui.shell(cx);
    let renaming = |cx: &mut TestAppContext| cx.update(|cx| shell.read(cx).renaming().is_some());

    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    ui.frame(cx);
    assert!(renaming(cx), "the field stays open");
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    // The session connects meanwhile: its focus request waits.
    let pane = ui.pane(first, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    ui.frame(cx);
    assert!(
        renaming(cx),
        "still open after the pane asked for the focus"
    );
    ui.input("prod", cx);
    ui.press("enter", cx);
    assert!(!renaming(cx));
    assert_eq!(ui.tabs(cx)[0].title().as_ref(), "prod");
    // The focus went back to the pane.
    ui.frame(cx);
    let focused = cx
        .update_window(ui.handle, |_, window, cx| {
            pane.read(cx).focus_handle(cx).contains_focused(window, cx)
        })
        .unwrap();
    assert!(focused, "the pane has the focus again");

    // Empty brings the host's title back; Enter on it unchanged keeps it
    // the host's (no title of the tab's own).
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    ui.press("backspace", cx);
    ui.press("enter", cx);
    assert_eq!(ui.tabs(cx)[0].custom_title(), None);
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-rename", cx);
    ui.press("enter", cx);
    assert_eq!(ui.tabs(cx)[0].title().as_ref(), "web");
    assert_eq!(ui.tabs(cx)[0].custom_title(), None, "not frozen");
    ui.finish(cx);
}

/// UI 4.4: while the active pane still chooses a host, Duplicate and the
/// splits are disabled, and Ctrl+Shift+D/E do nothing either.
#[gpui_kit::test]
fn a_pane_choosing_a_host_cannot_be_split(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let group = ui.group(id, cx);
    group.update(cx, |group, cx| {
        group.split(Direction::Row, cx);
    });
    ui.frame(cx);
    let picker = cx.update(|cx| group.read(cx).active_pane());
    assert!(cx.update(|cx| group.read(cx).shows_picker(picker)));
    let tab = ui.tabs(cx)[0].clone();
    for item in [
        "tab-menu-duplicate",
        "tab-menu-split-horizontal",
        "tab-menu-split-vertical",
    ] {
        ui.right_click(tab.element_id(), cx);
        ui.click(item, cx);
        assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 2, "{item}");
        assert_eq!(ui.tabs(cx).len(), 1, "{item}");
        // The menu stays open over a disabled item: Esc closes it.
        ui.press("escape", cx);
    }
    // The shortcuts, from the picker itself.
    group.update(cx, |group, cx| group.request_focus(cx));
    ui.frame(cx);
    assert_eq!(cx.update(|cx| group.read(cx).active_pane()), picker);
    ui.press("ctrl-shift-d", cx);
    ui.press("ctrl-shift-e", cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 2);
    assert!(cx.update(|cx| group.read(cx).shows_picker(picker)));
    ui.finish(cx);
}

/// UI 4.7, 12.5: Ctrl+Shift+W closes the active pane of a tab with splits,
/// then the tab.
#[gpui_kit::test]
fn ctrl_shift_w_closes_the_active_pane_then_the_tab(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let group = ui.group(id, cx);
    group.update(cx, |group, cx| {
        group.split(Direction::Row, cx);
    });
    ui.frame(cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 2);
    ui.press("ctrl-shift-w", cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 1);
    assert_eq!(ui.tabs(cx).len(), 1, "the tab stays");
    // Again from the shell's own focus (the tab strip): the same rule.
    group.update(cx, |group, cx| {
        group.split(Direction::Column, cx);
    });
    ui.frame(cx);
    let shell = ui.shell(cx);
    shell.update(cx, |shell, cx| shell.close_active(cx));
    ui.frame(cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 1);
    ui.press("ctrl-shift-w", cx);
    assert!(ui.tabs(cx).is_empty());
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.finish(cx);
}

/// UI 1.4, 2.4, 4.4: Alt + double click on a card, Alt + Connect in the host
/// panel and Duplicate in a New Window open the session in a new window of
/// the same size; Window > New Window opens an empty one.
#[gpui_kit::test]
fn alt_and_duplicate_in_a_new_window_open_windows(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let shell = ui.shell(cx);

    Ui::alt_click(ui.handle, &format!("host-card-{host}"), 2, cx);
    let windows = Ui::windows(cx);
    assert_eq!(windows.len(), 2, "a second window");
    assert!(ui.tabs(cx).is_empty(), "nothing opened here");
    let second = windows
        .into_iter()
        .find(|handle| handle.window_id() != ui.handle.window_id())
        .unwrap();
    let other = Ui::shell_of(second, cx);
    let tabs = cx.update(|cx| other.read(cx).sessions().tabs().to_vec());
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].host(), Some(host));
    // The size of the window it came from.
    let size_of = |handle: AnyWindowHandle, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| window.bounds().size)
            .unwrap()
    };
    assert_eq!(size_of(second, cx), size_of(ui.handle, cx));

    // Alt + Connect in the host panel.
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_host_panel(Some(host), window, cx)
        });
    })
    .unwrap();
    ui.frame(cx);
    Ui::alt_click(ui.handle, "host-connect", 1, cx);
    assert_eq!(Ui::windows(cx).len(), 3);
    assert!(ui.tabs(cx).is_empty());

    // Duplicate in a New Window.
    let id = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let tab = ui.tabs(cx)[0].clone();
    ui.right_click(tab.element_id(), cx);
    ui.click("tab-menu-duplicate-window", cx);
    assert_eq!(Ui::windows(cx).len(), 4);
    assert_eq!(ui.tabs(cx).len(), 1);
    assert_eq!(ui.top_tab(cx), TopTab::Session(id));

    // Window > New Window: an empty window.
    cx.update_window(ui.handle, |_, window, cx| {
        window.dispatch_action(Box::new(NewWindow), cx)
    })
    .unwrap();
    ui.frame(cx);
    let windows = Ui::windows(cx);
    assert_eq!(windows.len(), 5);
    let newest = Ui::shell_of(*windows.last().unwrap(), cx);
    assert!(cx.update(|cx| newest.read(cx).sessions().is_empty()));
    ui.finish(cx);
}

/// UI 1.4, 4.6, 13.3: closing a window with active sessions asks first;
/// without them it closes; locking covers every window.
#[gpui_kit::test]
fn windows_ask_before_closing_and_lock_together(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            windows::open_in_new_window(shell.state.clone(), window, cx, |shell, _, cx| {
                shell.open_session(SessionRequest::new(SessionKind::Local), cx);
            });
        });
    })
    .unwrap();
    ui.frame(cx);
    let second = *Ui::windows(cx).last().unwrap();
    Ui::frame_of(second, cx);
    let other = Ui::shell_of(second, cx);
    assert_eq!(cx.update(|cx| other.read(cx).active_sessions(cx)), 1);

    // It asks, and Cancel keeps the window.
    let closed = VisualTestContext::from_window(second, cx).simulate_close();
    assert!(!closed);
    let asking = cx
        .update_window(second, |_, window, cx| {
            let open = window.has_active_dialog(cx);
            window.close_all_dialogs(cx);
            open
        })
        .unwrap();
    assert!(asking, "the confirmation is open");
    assert_eq!(Ui::windows(cx).len(), 2);

    // Locking covers both windows (each shows the unlock screen).
    shell.update(cx, |shell, cx| shell.lock_app(cx));
    ui.frame(cx);
    Ui::frame_of(second, cx);
    for handle in Ui::windows(cx) {
        assert!(Ui::visible_in(handle, "lock-card", cx), "locked");
        assert!(!Ui::visible_in(handle, "shell", cx));
    }

    // Without active sessions a window closes without asking.
    let ended = cx.update(|cx| {
        let tab = other.read(cx).sessions().tabs()[0].id();
        other.read(cx).pane(tab).cloned().unwrap()
    });
    ended.update(cx, |pane, cx| {
        pane.set_phase(Phase::Ended(ExitStatus::Code(1)), None, cx)
    });
    let closed = VisualTestContext::from_window(second, cx).simulate_close();
    assert!(closed, "no question");
    assert!(
        !cx.update_window(second, |_, window, cx| window.has_active_dialog(cx))
            .unwrap()
    );
    ui.finish(cx);
}

/// The close button the top bar draws itself (client-side decorations on
/// Linux) does what the system close does (UI 1.4, 4.6): with active
/// sessions it asks, without them the last window saves its tabs for
/// Restore and closes.
#[gpui_kit::test]
fn the_drawn_close_button_asks_like_the_system_close(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let pane = ui.pane(id, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    cx.update_window(ui.handle, |_, window, cx| {
        crate::shell::request_close_window(window, cx)
    })
    .unwrap();
    let asking = cx
        .update_window(ui.handle, |_, window, cx| {
            let open = window.has_active_dialog(cx);
            window.close_all_dialogs(cx);
            open
        })
        .unwrap();
    assert!(asking, "the confirmation is open");
    assert_eq!(Ui::windows(cx).len(), 1);

    ui.set_phase(&pane, Phase::Ended(ExitStatus::Code(1)), cx);
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    cx.update_window(ui.handle, |_, window, cx| {
        crate::shell::request_close_window(window, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(Ui::windows(cx).is_empty(), "closed without a question");
    let saved = core.last_session().unwrap().expect("saved at close");
    assert_eq!(saved.windows[0].tabs.len(), 1);
    drop(core);
    ui.finish(cx);
}

/// UI 4.2: the top bar button shows and hides the terminal sidebar on the
/// right of the session area (the terminal narrows); Ctrl+Shift+F opens it
/// on Tools; it acts on the active pane's terminal.
#[gpui_kit::test]
fn the_terminal_sidebar_toggles_and_finds(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let pane = ui.pane(id, cx);
    ui.set_phase(&pane, Phase::Running, cx);
    let area = |cx: &mut TestAppContext| {
        cx.update_window(ui.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("session-area").bounds().size.width
        })
        .unwrap()
    };
    let full = area(cx);
    assert!(!ui.visible("terminal-sidebar", cx));
    ui.click("top-bar-terminal-sidebar", cx);
    assert!(ui.visible("terminal-sidebar", cx));
    assert_eq!(area(cx), full - crate::terminal_sidebar::WIDTH);
    let shell = ui.shell(cx);
    let sidebar = cx.update(|cx| shell.read(cx).terminal_sidebar().clone());
    let terminal = cx.update(|cx| pane.read(cx).terminal().cloned().unwrap());
    let target = cx.update(|cx| sidebar.read(cx).target().cloned());
    assert_eq!(
        target.map(|target| target.terminal_id()),
        Some(terminal.entity_id())
    );
    ui.click("top-bar-terminal-sidebar", cx);
    assert!(!ui.visible("terminal-sidebar", cx));
    assert_eq!(area(cx), full);

    // Ctrl+Shift+F from the terminal: Tools with the search focused.
    ui.press("ctrl-shift-f", cx);
    assert!(ui.visible("terminal-sidebar", cx));
    assert_eq!(
        cx.update(|cx| sidebar.read(cx).tab()),
        crate::terminal_sidebar::SidebarTab::Tools
    );
    // Not on Vaults.
    ui.press("ctrl-1", cx);
    assert!(!ui.visible("terminal-sidebar", cx));
    assert!(!ui.visible("top-bar-terminal-sidebar", cx));
    ui.finish(cx);
}

/// UI 6 (lead decision L5): the windows, tabs, panes with their shares and
/// the titles set with Rename are saved and come back with Restore: the
/// first window into the current one, the others as new windows; Restore
/// is offered once.
#[gpui_kit::test]
fn restore_brings_back_windows_tabs_panes_and_titles(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let shell = ui.shell(cx);
    // Window 1: "prod" = web | local (30 % / 70 %), then a quick connect.
    let prod = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let group = ui.group(prod, cx);
    let new_pane = group
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    group.update(cx, |group, cx| {
        group.open_in(
            new_pane,
            &PaneTarget::Local {
                shell_id: Some("pwsh".into()),
            },
            cx,
        );
        group.set_share(&SplitPath::default(), 0.3, cx);
    });
    shell.update(cx, |shell, cx| {
        shell.sessions.rename(prod, Some("prod"));
        cx.notify();
    });
    let quick = QuickTarget {
        username: Some("admin".into()),
        host: "192.0.2.30".into(),
        port: Some(2200),
    };
    ui.open_tab(
        SessionRequest::new(SessionKind::Ssh).quick(quick.clone()),
        cx,
    );
    // A new tab page is not saved.
    ui.click("tab-new", cx);
    // Window 2: a local terminal.
    cx.update_window(ui.handle, |_, window, cx| {
        shell.update(cx, |shell, cx| {
            windows::open_in_new_window(shell.state.clone(), window, cx, |shell, _, cx| {
                shell.open_session(SessionRequest::new(SessionKind::Local), cx);
            });
        });
    })
    .unwrap();
    ui.frame(cx);

    cx.update(|cx| restore::save_last_session(&ui.state, None, cx));
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    let saved = core.last_session().unwrap().expect("saved");
    assert_eq!(saved.windows.len(), 2);
    assert_eq!(saved.windows[0].tabs.len(), 2);
    assert_eq!(saved.windows[0].tabs[0].title.as_deref(), Some("prod"));
    assert_eq!(saved.windows[0].tabs[0].root.sessions().len(), 2);
    assert_eq!(saved.windows[1].tabs.len(), 1);
    drop(core);

    // The next launch: everything closed, the snapshot read at unlock.
    for handle in Ui::windows(cx) {
        if handle.window_id() != ui.handle.window_id() {
            let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        }
    }
    cx.run_until_parked();
    for tab in ui.tabs(cx) {
        shell.update(cx, |shell, cx| shell.close_session(tab.id(), cx));
    }
    ui.frame(cx);
    cx.update(|cx| {
        restore::forget_previous_launch(cx);
        restore::load_previous_launch(&ui.state, cx);
    });
    assert!(cx.update(|cx| restore::can_restore(cx)));
    let launches_before = ui.launches().len();

    ui.click("tab-new", cx);
    let page = ui.tabs(cx)[0].id();
    ui.click("new-tab-restore", cx);
    ui.frame(cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 2, "the page gave way to the restored tabs");
    assert!(tabs.iter().all(|tab| tab.id() != page));
    assert_eq!(tabs[0].title().as_ref(), "prod");
    assert_eq!(
        tabs[0].custom_title().map(|t| t.to_string()),
        Some("prod".into())
    );
    let restored = ui.group(tabs[0].id(), cx);
    let layout = cx.update(|cx| restored.read(cx).layout().clone());
    assert_eq!(layout.leaf_count(), 2);
    assert!(
        (layout.dividers()[0].ratio - 0.3).abs() < 0.01,
        "the shares"
    );
    let targets = cx.update(|cx| restored.read(cx).snapshot(cx).unwrap());
    assert_eq!(
        targets.leaves().into_iter().cloned().collect::<Vec<_>>(),
        vec![
            PaneTarget::Host {
                vault_id: cx.update(|cx| ui.state.read(cx).vault_id().unwrap()),
                record_id: host,
            },
            PaneTarget::Local {
                shell_id: Some("pwsh".into()),
            },
        ]
    );
    assert_eq!(tabs[1].kind(), SessionKind::Ssh);
    assert_eq!(tabs[1].host(), None);
    // The sessions connect: three panes here, one in the new window.
    let windows = Ui::windows(cx);
    assert_eq!(windows.len(), 2, "the second window again");
    let second = *windows
        .iter()
        .find(|handle| handle.window_id() != ui.handle.window_id())
        .unwrap();
    Ui::frame_of(second, cx);
    let other = Ui::shell_of(second, cx);
    let other_tabs = cx.update(|cx| other.read(cx).sessions().tabs().to_vec());
    assert_eq!(other_tabs.len(), 1);
    assert_eq!(other_tabs[0].kind(), SessionKind::Local);
    assert_eq!(ui.launches().len(), launches_before + 4);
    assert!(
        ui.launches()
            .contains(&OpenRequest::Ssh(SessionTarget::Quick {
                address: quick.host.clone(),
                port: quick.port,
                username: quick.username.clone(),
            }))
    );
    // Once per launch.
    assert!(!cx.update(|cx| restore::can_restore(cx)));
    ui.finish(cx);
}

/// UI 6 (lead decision L5): the last window saves its tabs when it closes,
/// and the lock saves them while the keys are there.
#[gpui_kit::test]
fn the_last_window_and_the_lock_save_the_tabs(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    let id = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let shell = ui.shell(cx);
    shell.update(cx, |shell, cx| shell.lock_app(cx));
    ui.frame(cx);
    assert!(
        core.last_session().is_err(),
        "locked: the record needs the keys"
    );
    let password = ui.password.clone();
    ui.state
        .update(cx, |state, cx| state.unlock(password, cx))
        .detach();
    ui.frame(cx);
    let saved = core.last_session().unwrap().expect("saved at lock");
    assert_eq!(saved.windows[0].tabs.len(), 1);

    // The window closes (the session ended: no question) and saves.
    let pane = ui.pane(id, cx);
    ui.set_phase(&pane, Phase::Ended(ExitStatus::Code(1)), cx);
    let second = ui.open_tab(SessionRequest::new(SessionKind::Local), cx);
    let pane = ui.pane(second, cx);
    ui.set_phase(&pane, Phase::Failed(Failure::Other), cx);
    let closed = VisualTestContext::from_window(ui.handle, cx).simulate_close();
    assert!(closed);
    cx.run_until_parked();
    let saved = core.last_session().unwrap().expect("saved at close");
    assert_eq!(saved.windows[0].tabs.len(), 2);
    drop(core);
    ui.finish(cx);
}

/// UI 4.7, 4.2: a tab with splits shows its active pane's host (unless
/// renamed), and the terminal sidebar acts on the active pane.
#[gpui_kit::test]
fn the_tab_follows_its_active_pane(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let group = ui.group(id, cx);
    let first = cx.update(|cx| group.read(cx).active_pane());
    let second = group
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    ui.frame(cx);
    // While the new pane chooses a host, the tab keeps the split pane's.
    let tab = |cx: &mut TestAppContext| ui.tabs(cx)[0].clone();
    assert_eq!(tab(cx).title().as_ref(), "web");
    assert!(
        ui.shell(cx)
            .read_with(cx, |shell, _| shell.pane(id).is_none())
    );
    group.update(cx, |group, cx| {
        group.open_in(second, &PaneTarget::Local { shell_id: None }, cx)
    });
    ui.frame(cx);
    assert_eq!(tab(cx).title().as_ref(), i18n::sessions::LOCAL_TERMINAL);
    assert_eq!(tab(cx).host(), None);
    assert_eq!(tab(cx).icon(), Lucide::Monitor);

    // The sidebar follows the active pane's terminal.
    ui.click("top-bar-terminal-sidebar", cx);
    let shell = ui.shell(cx);
    let sidebar = cx.update(|cx| shell.read(cx).terminal_sidebar().clone());
    let terminal_of = |pane: PaneId, cx: &mut TestAppContext| {
        cx.update(|cx| {
            group
                .read(cx)
                .session(pane)
                .and_then(|session| session.read(cx).terminal().cloned())
                .map(|terminal| terminal.entity_id())
        })
    };
    let target = |cx: &mut TestAppContext| {
        cx.update(|cx| sidebar.read(cx).target().map(|target| target.terminal_id()))
    };
    assert_eq!(target(cx), terminal_of(second, cx));

    group.update(cx, |group, cx| group.activate(first, cx));
    ui.frame(cx);
    assert_eq!(tab(cx).title().as_ref(), "web");
    assert_eq!(tab(cx).host(), Some(host));
    assert_eq!(target(cx), terminal_of(first, cx));
    assert_eq!(
        cx.update(|cx| sidebar
            .read(cx)
            .target()
            .and_then(|target| target.host_id())),
        Some(host)
    );
    // A renamed tab keeps its title whatever pane is active.
    shell.update(cx, |shell, cx| {
        shell.sessions.rename(id, Some("pair"));
        cx.notify();
    });
    group.update(cx, |group, cx| group.activate(second, cx));
    ui.frame(cx);
    assert_eq!(tab(cx).title().as_ref(), "pair");
    ui.finish(cx);
}

/// UI 2.4: Alt held while choosing Quick Connect in the card menu opens the
/// session in a new window.
#[gpui_kit::test]
fn alt_on_quick_connect_opens_a_new_window(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    ui.right_click(format!("host-card-{host}"), cx);
    assert!(ui.visible("menu-quick-connect", cx));
    Ui::alt_click(ui.handle, "menu-quick-connect", 1, cx);
    let windows = Ui::windows(cx);
    assert_eq!(windows.len(), 2);
    assert!(ui.tabs(cx).is_empty());
    let other = Ui::shell_of(*windows.last().unwrap(), cx);
    let tabs = cx.update(|cx| other.read(cx).sessions().tabs().to_vec());
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].host(), Some(host));
    ui.finish(cx);
}

/// UI 6: a saved host removed since the last launch is not opened (its
/// sibling pane takes the space); when nothing can be opened, a toast says
/// so and the new tab page stays.
#[gpui_kit::test]
fn restore_leaves_out_removed_hosts(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let gone = ui.save_host(json!({"label": "old", "address": "192.0.2.50"}), cx);
    let id = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(gone), cx);
    let group = ui.group(id, cx);
    let local = group
        .update(cx, |group, cx| group.split(Direction::Column, cx))
        .unwrap();
    group.update(cx, |group, cx| {
        group.open_in(local, &PaneTarget::Local { shell_id: None }, cx)
    });
    ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(gone), cx);
    cx.update(|cx| restore::save_last_session(&ui.state, None, cx));
    let shell = ui.shell(cx);
    for tab in ui.tabs(cx) {
        shell.update(cx, |shell, cx| shell.close_session(tab.id(), cx));
    }
    ui.state
        .update(cx, |state, cx| state.delete_records(vec![gone], cx))
        .unwrap();
    cx.update(|cx| {
        restore::forget_previous_launch(cx);
        restore::load_previous_launch(&ui.state, cx);
    });
    ui.frame(cx);

    ui.click("tab-new", cx);
    ui.click("new-tab-restore", cx);
    let tabs = ui.tabs(cx);
    assert_eq!(tabs.len(), 1, "only the tab with a local pane is back");
    assert_eq!(tabs[0].kind(), SessionKind::Local);
    let restored = ui.group(tabs[0].id(), cx);
    assert_eq!(cx.update(|cx| restored.read(cx).pane_count()), 1);
    assert!(
        !ui.launches().iter().skip(3).any(|request| matches!(
            request,
            OpenRequest::Ssh(SessionTarget::Host { record_id, .. }) if *record_id == gone
        )),
        "the removed host is not opened"
    );

    // Nothing to open: a toast, and the page stays.
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    core.last_session_save(vec![shellpp_core::service::last_session::SavedWindow {
        tabs: vec![shellpp_core::service::last_session::SavedTab {
            title: None,
            root: shellpp_core::service::last_session::SavedPane::Leaf {
                session: shellpp_core::service::last_session::SavedSession::Host(
                    shellpp_core::service::dto::RecordRef {
                        vault_id: cx.update(|cx| ui.state.read(cx).vault_id().unwrap()),
                        record_id: gone,
                    },
                ),
            },
        }],
    }])
    .unwrap();
    drop(core);
    cx.update(|cx| {
        restore::forget_previous_launch(cx);
        restore::load_previous_launch(&ui.state, cx);
    });
    ui.click("tab-new", cx);
    let page = ui.tabs(cx).last().unwrap().id();
    ui.click("new-tab-restore", cx);
    assert!(
        ui.tabs(cx).iter().any(|tab| tab.id() == page),
        "the page stays"
    );
    let toasts = cx.update(|cx| shell.read(cx).toasts().read(cx).items().len());
    assert!(toasts > 0, "a toast says nothing could be restored");
    // A notification (UI 1.5): the notification center keeps it.
    let noted = cx.update(|cx| {
        let list = crate::shell::notifications::Notifications::global(cx);
        list.read(cx)
            .entries()
            .iter()
            .any(|entry| entry.message().as_ref() == crate::shell::strings::RESTORE_NOTHING)
    });
    assert!(noted, "the notification center lists it");
    ui.finish(cx);
}

/// UI 6: the search covers the open tabs of every window; choosing one in
/// another window brings that window's tab forward.
#[gpui_kit::test]
fn ctrl_k_activates_a_tab_of_another_window(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    Ui::alt_click(ui.handle, &format!("host-card-{host}"), 2, cx);
    let second = *Ui::windows(cx)
        .iter()
        .find(|handle| handle.window_id() != ui.handle.window_id())
        .unwrap();
    let other = Ui::shell_of(second, cx);
    let tab = cx.update(|cx| other.read(cx).sessions().tabs()[0].id());
    // That window shows Vaults meanwhile.
    other.update(cx, |shell, cx| shell.set_top_tab(TopTab::Vaults, cx));
    Ui::frame_of(second, cx);

    ui.press("ctrl-k", cx);
    ui.input("web", cx);
    ui.press("enter", cx);
    Ui::frame_of(second, cx);
    assert!(!ui.visible("search-overlay", cx));
    assert_eq!(
        cx.update(|cx| other.read(cx).top_tab()),
        TopTab::Session(tab)
    );
    assert!(ui.tabs(cx).is_empty(), "no new tab here");
    ui.finish(cx);
}

/// D92, D94: a workspace whose host was deleted is marked "1 host not
/// found" on the new tab page (named in the tooltip from the names the
/// workspace saved); the mark goes when the host is back.
#[gpui_kit::test]
fn a_workspace_marks_hosts_that_are_gone(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    let vault = cx.update(|cx| ui.state.read(cx).vault_id().unwrap());
    let link = json!({"vault_id": vault.to_string(), "record_id": host.to_string()});
    let saved = core
        .workspace_save(shellpp_core::service::snippets::RecordInput {
            vault_id: vault,
            record_id: None,
            data: json!({"name": "Prod", "tabs": [{"root": {"session": {"kind": "ssh", "host": link}}}]})
                .as_object()
                .cloned()
                .unwrap(),
        })
        .unwrap();
    let id = saved.record.record_id;
    ui.state.update(cx, |state, cx| state.reload_records(cx));
    ui.click("tab-new", cx);
    assert!(ui.visible(format!("new-tab-workspace-{id}"), cx));
    assert!(!ui.visible(format!("new-tab-workspace-missing-{id}"), cx));

    // The host is deleted: the mark shows, with the saved name.
    ui.state.update(cx, |state, cx| {
        state.delete_records(vec![host], cx).unwrap();
    });
    ui.frame(cx);
    assert!(ui.visible(format!("new-tab-workspace-missing-{id}"), cx));
    let item = cx.update(|cx| {
        ui.state
            .read(cx)
            .workspaces()
            .iter()
            .find(|workspace| workspace.id == id)
            .cloned()
            .unwrap()
    });
    assert_eq!(&*item.missing, [SharedString::from("web")]);
    assert_eq!(
        crate::new_tab::strings::missing_hosts_tooltip(&item.missing).as_ref(),
        "Not found here: web"
    );

    // Undo brings it back: no mark.
    ui.state.update(cx, |state, cx| {
        state.restore_records(vec![host], cx).unwrap();
    });
    ui.frame(cx);
    assert!(!ui.visible(format!("new-tab-workspace-missing-{id}"), cx));
    drop(core);
    ui.finish(cx);
}

/// UI 4.5, 6 (D81): "Create a workspace" saves this window's tabs to saved
/// hosts and local terminals (a quick connection is left out) under a
/// name; the page lists it, a click opens its tabs, × removes it.
#[gpui_kit::test]
fn a_workspace_is_created_opened_and_removed(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(web(), cx);
    let prod = ui.open_tab(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let group = ui.group(prod, cx);
    let new_pane = group
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    group.update(cx, |group, cx| {
        group.open_in(new_pane, &PaneTarget::Local { shell_id: None }, cx);
    });
    ui.open_tab(
        SessionRequest::new(SessionKind::Ssh).quick(QuickTarget {
            username: None,
            host: "192.0.2.31".into(),
            port: None,
        }),
        cx,
    );
    ui.click("tab-new", cx);
    ui.click("new-tab-workspace", cx);
    assert!(ui.visible("workspace-dialog", cx));
    // No name: the dialog stays and says so.
    ui.click("workspace-create", cx);
    assert!(ui.visible("workspace-dialog", cx));
    assert!(ui.visible("workspace-name-error", cx));
    ui.click("workspace-name", cx);
    ui.input("Prod", cx);
    ui.click("workspace-create", cx);
    assert!(!ui.visible("workspace-dialog", cx));

    let core = cx.update(|cx| ui.state.read(cx).core().cloned().unwrap());
    let vault = cx.update(|cx| ui.state.read(cx).vault_id().unwrap());
    let saved = core.workspaces_list(vault).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Prod");
    assert_eq!(saved[0].tabs.len(), 1, "the quick connection is left out");
    assert_eq!(saved[0].tabs[0].root.sessions().len(), 2);
    let id = saved[0].record.record_id;

    // Rename keeps the tabs.
    ui.click(format!("new-tab-workspace-rename-{id}"), cx);
    assert!(ui.visible("workspace-dialog", cx));
    ui.click("workspace-name", cx);
    ui.press("ctrl-a", cx);
    ui.input("Production", cx);
    ui.click("workspace-create", cx);
    let saved = core.workspaces_list(vault).unwrap();
    assert_eq!(saved[0].name, "Production");
    assert_eq!(saved[0].tabs.len(), 1);

    // A click on the workspace opens its tab in place of the page.
    let before = ui.tabs(cx).len();
    ui.click(format!("new-tab-workspace-{id}"), cx);
    assert_eq!(ui.tabs(cx).len(), before, "the page is replaced");
    let opened = ui.tabs(cx).last().unwrap().id();
    let opened_group = ui.group(opened, cx);
    assert_eq!(cx.update(|cx| opened_group.read(cx).pane_count()), 2);

    // × removes it at once; the toast's Undo brings it back (D90).
    ui.click("tab-new", cx);
    ui.click(format!("new-tab-workspace-remove-{id}"), cx);
    assert!(core.workspaces_list(vault).unwrap().is_empty());
    assert!(!ui.visible(format!("new-tab-workspace-{id}"), cx));
    assert!(ui.visible("toast-undo", cx));
    ui.click("toast-undo", cx);
    let back = core.workspaces_list(vault).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].record.record_id, id);
    assert_eq!(back[0].name, "Production");
    assert_eq!(back[0].tabs.len(), 1, "with its tabs");
    assert!(ui.visible(format!("new-tab-workspace-{id}"), cx));
    // Removed again, it stays removed.
    ui.click(format!("new-tab-workspace-remove-{id}"), cx);
    assert!(core.workspaces_list(vault).unwrap().is_empty());
    drop(core);
    ui.finish(cx);
}

/// UI 9.4, D81: a snippet run in a split tab: one pane per host (three in a
/// row), and each pane types the script once its session first runs.
#[gpui_kit::test]
fn a_split_run_types_the_script_in_every_pane(cx: &mut TestAppContext) {
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<u8>>);
    impl crate::terminal::TerminalSink for Sink {
        fn send(&self, bytes: Vec<u8>) {
            self.0.lock().unwrap().extend_from_slice(&bytes);
        }
        fn resize(&self, _: crate::terminal::TerminalSize) {}
    }

    let ui = Ui::open(cx);
    let hosts: Vec<Uuid> = ["192.0.2.41", "192.0.2.42", "192.0.2.43"]
        .into_iter()
        .map(|address| ui.save_host(json!({"address": address}), cx))
        .collect();
    let shell = ui.shell(cx);
    let id = shell
        .update(cx, |shell, cx| {
            shell.open_split_run("uptime".into(), &hosts, "uptime".into(), cx)
        })
        .expect("a tab");
    ui.frame(cx);
    let group = ui.group(id, cx);
    assert_eq!(cx.update(|cx| group.read(cx).pane_count()), 3);
    let sessions: Vec<Entity<SessionPane>> =
        cx.update(|cx| group.read(cx).sessions().into_iter().cloned().collect());
    let terminals = ui.record.terminals();
    let sinks: Vec<std::sync::Arc<Sink>> = terminals
        .iter()
        .rev()
        .take(3)
        .map(|terminal| {
            let sink = std::sync::Arc::new(Sink::default());
            terminal.set_sink(Some(sink.clone()));
            sink
        })
        .collect();
    for session in &sessions {
        session.update(cx, |session, cx| {
            session.set_phase(Phase::Running, None, cx)
        });
    }
    // No prompt with bracketed paste on the fake terminal: typed after the
    // wait.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(4));
    cx.run_until_parked();
    for sink in &sinks {
        assert!(
            sink.0.lock().unwrap().ends_with(b"uptime\r"),
            "the script and Enter"
        );
    }
    // Typed once: a later run (a reconnect) does not type it again.
    for session in &sessions {
        session.update(cx, |session, cx| {
            session.set_phase(Phase::Reconnecting(ConnectStep::Connecting), None, cx);
            session.set_phase(Phase::Running, None, cx);
        });
    }
    for sink in &sinks {
        assert_eq!(sink.0.lock().unwrap().as_slice(), b"uptime\r");
    }
    ui.finish(cx);
}
