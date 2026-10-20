//! Headless tests of the panes of a session tab (UI 4.6, 4.7) with the fake
//! session layer: a temporary profile, no network, no PTY, no keys. The
//! group lives in a test window of its own; elements are found by element
//! id, scoped to a pane (`pane-<id>`) where every pane has the same ids.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, Bounds, Entity, Render, SharedString, Subscription, TestAppContext,
    WindowBounds, WindowOptions, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;

use super::*;
use crate::app;
use crate::hosts::QuickTarget;
use crate::session_ui::fake::{Call, FakeLauncher, Record};
use crate::session_ui::{ConnectStep, ExitStatus, Failure};
use crate::sessions::OpenRequest;
use crate::testing::TestProfile;
use crate::theme::{self, Appearance};

/// The window's root: the group, filling the window, and the events it
/// emitted.
struct Host {
    group: Entity<PaneGroup>,
    _subscription: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.group.clone())
    }
}

struct Tab {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    group: Entity<PaneGroup>,
    record: Record,
    events: Rc<RefCell<Vec<PaneGroupEvent>>>,
}

const WIDTH: f32 = 1200.;
const HEIGHT: f32 = 800.;

impl Tab {
    /// A window whose group starts with one local terminal.
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(|_, _| PaneTarget::Local { shell_id: None }, cx)
    }

    /// A window whose group starts with `first(state)`.
    fn open_with(
        first: impl FnOnce(&Entity<AppState>, &mut TestAppContext) -> PaneTarget,
        cx: &mut TestAppContext,
    ) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
        });
        let record = FakeLauncher::install(cx);
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let target = first(&state, cx);
        let events = Rc::new(RefCell::new(Vec::new()));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(WIDTH), px(HEIGHT)),
            ))),
            ..Default::default()
        };
        let group_slot = Rc::new(RefCell::new(None));
        let handle = {
            let state = state.clone();
            let events = events.clone();
            let slot = group_slot.clone();
            cx.update(|cx| {
                gpui_kit::open_window(options, cx, move |_, cx| {
                    let group = cx.new(|cx| PaneGroup::open(state, &target, cx));
                    *slot.borrow_mut() = Some(group.clone());
                    cx.new(|cx| Host {
                        _subscription: cx.subscribe(&group, move |_, _, event, _| {
                            events.borrow_mut().push(event.clone())
                        }),
                        group,
                    })
                })
            })
            .expect("test window")
            .0
        };
        let group = group_slot.borrow_mut().take().expect("group");
        let tab = Self {
            state,
            handle,
            group,
            record,
            events,
        };
        tab.frame(cx);
        tab
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
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

    /// Clicks element `id` inside pane `pane`.
    fn click_in(&self, pane: PaneId, id: &str, cx: &mut TestAppContext) {
        let scope = PaneGroup::element_id(pane);
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.within(scope.clone()).click(id.clone(), cx);
        })
        .unwrap();
        self.frame(cx);
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .is_some_and(|element| element.visible())
        })
        .unwrap()
    }

    /// Whether element `id` is visible inside pane `pane`.
    fn visible_in(&self, pane: PaneId, id: &str, cx: &mut TestAppContext) -> bool {
        let scope = PaneGroup::element_id(pane);
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            if window.try_find(scope.clone()).is_none() {
                return false;
            }
            window
                .within(scope.clone())
                .try_find(id.clone())
                .is_some_and(|element| element.visible())
        })
        .unwrap()
    }

    /// The on-screen bounds of element `id`.
    fn bounds(&self, id: &str, cx: &mut TestAppContext) -> Bounds<Pixels> {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.find(id.clone()).bounds()
        })
        .unwrap()
    }

    fn drag(&self, from: gpui::Point<Pixels>, to: gpui::Point<Pixels>, cx: &mut TestAppContext) {
        cx.update_window(self.handle, move |_, window, cx| window.drag(from, to, cx))
            .unwrap();
        self.frame(cx);
    }

    fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&PaneGroup, &App) -> R) -> R {
        cx.update(|cx| f(self.group.read(cx), cx))
    }

    fn update<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut PaneGroup, &mut Context<PaneGroup>) -> R,
    ) -> R {
        let result = self.group.update(cx, f);
        self.frame(cx);
        result
    }

    fn active(&self, cx: &mut TestAppContext) -> PaneId {
        self.read(cx, |group, _| group.active_pane())
    }

    fn panes(&self, cx: &mut TestAppContext) -> Vec<PaneId> {
        self.read(cx, |group, _| group.pane_ids())
    }

    fn session(&self, pane: PaneId, cx: &mut TestAppContext) -> Entity<SessionPane> {
        self.read(cx, |group, _| group.session(pane).cloned())
            .expect("a session in the pane")
    }

    fn set_phase(&self, pane: PaneId, phase: Phase, cx: &mut TestAppContext) {
        let session = self.session(pane, cx);
        session.update(cx, |session, cx| session.set_phase(phase, None, cx));
        self.frame(cx);
    }

    /// Splits the active pane and opens a local terminal in the new one.
    fn split_local(&self, direction: Direction, cx: &mut TestAppContext) -> PaneId {
        let id = self
            .update(cx, |group, cx| group.split(direction, cx))
            .expect("split");
        self.update(cx, |group, cx| {
            group.open_in(id, &PaneTarget::Local { shell_id: None }, cx)
        });
        id
    }

    /// Whether the keyboard focus is in pane `pane`'s session.
    fn focused(&self, pane: PaneId, cx: &mut TestAppContext) -> bool {
        let session = self.session(pane, cx);
        cx.update_window(self.handle, |_, window, cx| {
            session
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
        .unwrap()
    }

    fn launches(&self) -> usize {
        self.record
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Launch(_)))
            .count()
    }

    fn events(&self) -> Vec<PaneGroupEvent> {
        self.events.borrow().clone()
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.group.update(cx, |group, cx| group.release(cx));
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// A saved host in the personal vault (generated, documentation address).
fn save_host(state: &Entity<AppState>, cx: &mut TestAppContext) -> PaneTarget {
    state.update(cx, |state, cx| {
        let saved = state
            .save_host(
                None,
                json!({"label": "web", "address": "192.0.2.10", "ssh": {"username": "tester"}})
                    .as_object()
                    .cloned()
                    .unwrap(),
                cx,
            )
            .unwrap();
        PaneTarget::Host {
            vault_id: saved.vault_id,
            record_id: saved.record_id,
        }
    })
}

/// UI 4.7: Ctrl+Shift+D and Ctrl+Shift+E (and the actions) split the active
/// pane in two halves; the new pane shows the picker with the split pane's
/// target first, becomes active and has the keyboard focus; choosing opens
/// a session in it.
#[gpui_kit::test]
fn splits_by_keys_and_actions(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let first = tab.active(cx);
    assert_eq!(tab.launches(), 1);
    tab.set_phase(first, Phase::Running, cx);
    assert!(
        !tab.visible(&format!("pane-{first}-active"), cx),
        "one pane: no mark"
    );

    // Ctrl+Shift+D: a pane to the right.
    tab.press("ctrl-shift-d", cx);
    let panes = tab.panes(cx);
    assert_eq!(panes.len(), 2);
    let right = panes[1];
    assert_eq!(tab.active(cx), right);
    assert!(tab.read(cx, |group, _| group.shows_picker(right)));
    assert!(tab.visible_in(right, "pane-picker", cx));
    assert!(tab.visible(&format!("pane-{right}-active"), cx));
    assert!(!tab.visible(&format!("pane-{first}-active"), cx));
    let left_bounds = tab.bounds(&format!("pane-{first}"), cx);
    let right_bounds = tab.bounds(&format!("pane-{right}"), cx);
    assert_eq!(left_bounds.size.width, px(WIDTH / 2.));
    assert_eq!(right_bounds.origin.x, px(WIDTH / 2.));
    assert_eq!(right_bounds.size.height, px(HEIGHT));
    // The picker has the focus; Enter opens the split pane's target.
    tab.press("enter", cx);
    assert!(!tab.read(cx, |group, _| group.shows_picker(right)));
    assert_eq!(tab.launches(), 2);
    assert_eq!(
        tab.record.calls().last(),
        Some(&Call::Launch(OpenRequest::Local { shell_id: None }))
    );
    assert!(tab.read(cx, |group, cx| {
        group
            .session(right)
            .is_some_and(|session| session.read(cx).is_started())
    }));

    // Ctrl+Shift+E from the right pane: a pane below it.
    tab.press("ctrl-shift-e", cx);
    let below = tab.active(cx);
    assert_eq!(tab.panes(cx), vec![first, right, below]);
    let below_bounds = tab.bounds(&format!("pane-{below}"), cx);
    assert_eq!(below_bounds.origin, point(px(WIDTH / 2.), px(HEIGHT / 2.)));
    // Esc in the picker undoes the split: the right pane takes the space.
    tab.press("escape", cx);
    assert_eq!(tab.panes(cx), vec![first, right]);
    assert_eq!(tab.active(cx), right);
    assert_eq!(tab.launches(), 2);

    // The actions do the same.
    tab.update(cx, |group, cx| group.split(Direction::Column, cx));
    assert_eq!(tab.panes(cx).len(), 3);
    let layout = tab.read(cx, |group, _| group.layout().clone());
    assert_eq!(layout.dividers()[1].direction, Direction::Column);
    tab.finish(cx);
}

/// UI 4.7: the new pane's picker lists the split pane's host first, so
/// Enter opens another session to the same saved host.
#[gpui_kit::test]
fn a_new_pane_offers_the_split_host_first(cx: &mut TestAppContext) {
    let tab = Tab::open_with(save_host, cx);
    let host = tab
        .read(cx, |group, cx| group.active_target(cx))
        .expect("host");
    assert!(host.host_id().is_some());
    let new = tab
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    let record_id = host.host_id().unwrap();
    assert!(tab.visible_in(new, &format!("pane-picker-row-origin-{record_id}"), cx));
    tab.press("enter", cx);
    assert_eq!(
        tab.record.calls().last(),
        Some(&Call::Launch(host.open_request()))
    );
    assert_eq!(tab.read(cx, |group, cx| group.target(new, cx)), Some(host));
    tab.finish(cx);
}

/// Dividers move with the pointer and keep every pane at least
/// `PANE_MIN` wide.
#[gpui_kit::test]
fn dividers_drag_within_bounds(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let first = tab.active(cx);
    let right = tab.split_local(Direction::Row, cx);
    let root = SplitPath::default();
    let handle = tab.bounds("pane-divider-r", cx);
    assert_eq!(handle.center().x, px(WIDTH / 2.));
    // To 300 px: the share follows the pointer.
    tab.drag(handle.center(), point(px(300.), handle.center().y), cx);
    let share = tab.read(cx, |group, _| group.layout().dividers()[0].ratio);
    assert!((share - 300. / WIDTH).abs() < 0.01, "{share}");
    assert_eq!(
        tab.bounds(&format!("pane-{first}"), cx).size.width,
        px(WIDTH * share)
    );
    // Past the left edge: the left pane keeps its minimum.
    let handle = tab.bounds("pane-divider-r", cx);
    tab.drag(handle.center(), point(px(10.), handle.center().y), cx);
    let share = tab.read(cx, |group, _| group.layout().dividers()[0].ratio);
    assert!((share * WIDTH - f32::from(PANE_MIN)).abs() < 0.5, "{share}");
    // Past the right edge: the right pane keeps its minimum.
    let handle = tab.bounds("pane-divider-r", cx);
    tab.drag(
        handle.center(),
        point(px(WIDTH - 1.), handle.center().y),
        cx,
    );
    let width = tab.bounds(&format!("pane-{right}"), cx).size.width;
    assert!(
        (f32::from(width) - f32::from(PANE_MIN)).abs() < 0.5,
        "{width:?}"
    );
    // A nested horizontal divider moves its own split only.
    tab.update(cx, |group, cx| group.activate(first, cx));
    let below = tab.split_local(Direction::Column, cx);
    let nested = tab.bounds("pane-divider-r-0", cx);
    tab.drag(nested.center(), point(nested.center().x, px(200.)), cx);
    let dividers = tab.read(cx, |group, _| group.layout().dividers());
    assert!((dividers[1].ratio - 200. / HEIGHT).abs() < 0.01);
    assert!((dividers[0].ratio * WIDTH - (WIDTH - f32::from(PANE_MIN))).abs() < 0.5);
    assert!(tab.visible(&format!("pane-{below}"), cx));
    // The API keeps the same bounds.
    assert!(tab.update(cx, |group, cx| group.set_share(&root, 0.5, cx)));
    tab.update(cx, |group, cx| group.set_share(&root, 0.0, cx));
    let share = tab.read(cx, |group, _| group.layout().dividers()[0].ratio);
    assert!((share * WIDTH - f32::from(PANE_MIN)).abs() < 0.5);
    tab.finish(cx);
}

/// UI 4.7: a click in a pane and Ctrl+Alt+arrows (the nearest pane on
/// screen; nothing at the edge) move the active pane and the focus.
#[gpui_kit::test]
fn click_and_ctrl_alt_arrows_move_the_focus(cx: &mut TestAppContext) {
    // 1 | 2
    // --+--
    // 3 | 4
    let tab = Tab::open(cx);
    let one = tab.active(cx);
    let two = tab.split_local(Direction::Row, cx);
    let four = tab.split_local(Direction::Column, cx);
    tab.update(cx, |group, cx| group.activate(one, cx));
    let three = tab.split_local(Direction::Column, cx);
    for pane in [one, two, three, four] {
        tab.set_phase(pane, Phase::Running, cx);
    }
    let focused = |tab: &Tab, pane: PaneId, cx: &mut TestAppContext| tab.focused(pane, cx);
    assert_eq!(tab.active(cx), three);
    assert!(focused(&tab, three, cx));
    tab.press("ctrl-alt-right", cx);
    assert_eq!(tab.active(cx), four);
    assert!(focused(&tab, four, cx));
    tab.press("ctrl-alt-up", cx);
    assert_eq!(tab.active(cx), two);
    tab.press("ctrl-alt-up", cx);
    assert_eq!(tab.active(cx), two, "the edge of the tab: nothing");
    tab.press("ctrl-alt-left", cx);
    assert_eq!(tab.active(cx), one);
    assert!(focused(&tab, one, cx));
    tab.press("ctrl-alt-down", cx);
    assert_eq!(tab.active(cx), three);
    // A click in another pane.
    tab.click(&format!("pane-{two}"), cx);
    assert_eq!(tab.active(cx), two);
    assert!(focused(&tab, two, cx));
    assert!(tab.visible(&format!("pane-{two}-active"), cx));
    assert!(
        tab.events().contains(&PaneGroupEvent::Changed),
        "the tab follows the active pane"
    );
    tab.finish(cx);
}

/// UI 4.7: a session that connects (or asks) in another pane leaves the
/// keyboard focus in the active pane.
#[gpui_kit::test]
fn only_the_active_pane_takes_the_focus(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let one = tab.active(cx);
    let two = tab.split_local(Direction::Row, cx);
    tab.set_phase(two, Phase::Running, cx);
    assert!(tab.focused(two, cx));
    // Pane 1 connects in the background.
    tab.set_phase(one, Phase::Running, cx);
    assert_eq!(tab.active(cx), two);
    assert!(tab.focused(two, cx), "the focus stays in the active pane");
    assert!(!tab.focused(one, cx));
    // Moving there gives it the focus.
    tab.press("ctrl-alt-left", cx);
    assert_eq!(tab.active(cx), one);
    assert!(tab.focused(one, cx));
    tab.finish(cx);
}

/// UI 4.6, 4.7: exit status 0 closes the pane and its sibling takes the
/// space; a non-zero exit and a drop keep their overlay in their pane; the
/// last pane closing empties the tab.
#[gpui_kit::test]
fn exits_and_drops_per_pane(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let one = tab.active(cx);
    let two = tab.split_local(Direction::Row, cx);
    let three = tab.split_local(Direction::Column, cx);
    for pane in [one, two, three] {
        tab.set_phase(pane, Phase::Running, cx);
    }
    // Non-zero exit in 2: the overlay stays in pane 2.
    tab.set_phase(two, Phase::Ended(ExitStatus::Code(2)), cx);
    assert!(tab.visible_in(two, "session-ended", cx));
    assert!(!tab.visible_in(one, "session-ended", cx));
    assert!(!tab.visible_in(three, "session-ended", cx));
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Ended),
        "an ended session is worse than connected"
    );
    // A drop in 3: its overlay, dimmed terminal, the tab's dot is a drop.
    tab.set_phase(three, Phase::Dropped(Failure::ConnectionLost), cx);
    assert!(tab.visible_in(three, "session-dropped", cx));
    assert!(!tab.visible_in(one, "session-dropped", cx));
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Dropped)
    );
    tab.set_phase(three, Phase::Reconnecting(ConnectStep::Connecting), cx);
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Connecting)
    );
    // Close on the ended pane closes that pane only.
    tab.click_in(two, "session-ended-close", cx);
    assert_eq!(tab.panes(cx), vec![one, three]);
    // Pane 3 now fills the right half.
    let bounds = tab.bounds(&format!("pane-{three}"), cx);
    assert_eq!(bounds.size.height, px(HEIGHT));
    assert_eq!(bounds.origin.x, px(WIDTH / 2.));
    // Exit 0 in 1: pane 3 takes the whole tab and becomes active.
    tab.update(cx, |group, cx| group.activate(one, cx));
    tab.set_phase(one, Phase::Closed, cx);
    assert_eq!(tab.panes(cx), vec![three]);
    assert_eq!(tab.active(cx), three);
    assert_eq!(
        tab.bounds(&format!("pane-{three}"), cx).size.width,
        px(WIDTH)
    );
    assert!(!tab.events().contains(&PaneGroupEvent::Empty));
    // The last one: the tab closes (the shell applies UI 4.6).
    tab.set_phase(three, Phase::Running, cx);
    tab.set_phase(three, Phase::Closed, cx);
    assert!(tab.events().contains(&PaneGroupEvent::Empty));
    assert_eq!(tab.read(cx, |group, _| group.pane_count()), 0);
    assert!(tab.panes(cx).is_empty());
    // Every closed pane's session was told to end.
    let closes = tab
        .record
        .calls()
        .iter()
        .filter(|call| **call == Call::Close)
        .count();
    assert_eq!(closes, 3);
    tab.finish(cx);
}

/// UI 4.7: closing the active pane (Ctrl+Shift+W in the shell) gives its
/// space to the sibling; closing the last one empties the tab.
#[gpui_kit::test]
fn closing_the_active_pane(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let one = tab.active(cx);
    let two = tab.split_local(Direction::Row, cx);
    tab.set_phase(one, Phase::Running, cx);
    tab.set_phase(two, Phase::Running, cx);
    assert!(tab.read(cx, |group, _| group.has_splits()));
    assert_eq!(
        tab.update(cx, |group, cx| group.close_active_pane(cx)),
        PaneClosed::Closed
    );
    assert_eq!(tab.panes(cx), vec![one]);
    assert_eq!(tab.active(cx), one);
    assert!(tab.focused(one, cx));
    assert!(!tab.read(cx, |group, _| group.has_splits()));
    assert!(!tab.events().contains(&PaneGroupEvent::Empty));
    assert_eq!(
        tab.update(cx, |group, cx| group.close_pane(two, cx)),
        PaneClosed::NotFound
    );
    assert_eq!(
        tab.update(cx, |group, cx| group.close_active_pane(cx)),
        PaneClosed::TabEmpty
    );
    assert!(tab.events().contains(&PaneGroupEvent::Empty));
    tab.finish(cx);
}

/// The tab's title is the active pane's; while a new pane chooses a host,
/// the pane that was split; the dot is the worst state of the panes.
#[gpui_kit::test]
fn the_tab_shows_the_active_pane_and_the_worst_state(cx: &mut TestAppContext) {
    let tab = Tab::open_with(
        |_, _| {
            PaneTarget::Quick(QuickTarget {
                username: Some("tester".into()),
                host: "192.0.2.20".into(),
                port: None,
            })
        },
        cx,
    );
    let one = tab.active(cx);
    let title = |tab: &Tab, cx: &mut TestAppContext| {
        tab.read(cx, |group, cx| {
            group.info(cx).map(|info| info.title().clone())
        })
    };
    assert_eq!(title(&tab, cx).as_deref(), Some("tester@192.0.2.20"));
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Connecting)
    );
    tab.set_phase(one, Phase::Running, cx);
    // The first run of a quick connect is reported (the save offer).
    assert!(
        tab.events()
            .iter()
            .any(|event| matches!(event, PaneGroupEvent::Connected(PaneTarget::Quick(_))))
    );
    let two = tab
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    assert_eq!(title(&tab, cx).as_deref(), Some("tester@192.0.2.20"));
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Connected)
    );
    tab.update(cx, |group, cx| {
        group.open_in(two, &PaneTarget::Local { shell_id: None }, cx)
    });
    assert_eq!(
        title(&tab, cx).as_deref(),
        Some(crate::i18n::sessions::LOCAL_TERMINAL)
    );
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Connecting)
    );
    tab.set_phase(two, Phase::Running, cx);
    tab.set_phase(one, Phase::Dropped(Failure::ConnectionLost), cx);
    assert_eq!(
        tab.read(cx, |group, cx| group.worst_status(cx)),
        Some(PaneStatus::Dropped)
    );
    tab.update(cx, |group, cx| group.activate(one, cx));
    assert_eq!(title(&tab, cx).as_deref(), Some("tester@192.0.2.20"));
    assert_eq!(
        tab.read(cx, |group, cx| group.active_target(cx)),
        Some(PaneTarget::Quick(QuickTarget {
            username: Some("tester".into()),
            host: "192.0.2.20".into(),
            port: None,
        }))
    );
    tab.finish(cx);
}

/// Splits stop at the saved-tab limits and at the pane minimum.
#[gpui_kit::test]
fn splits_stop_at_the_limits(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    // Halving to the right: 1200, 600, 300, 150 px; a 150 px pane cannot
    // hold two 128 px panes.
    for _ in 0..3 {
        assert!(
            tab.update(cx, |group, cx| group.split(Direction::Row, cx))
                .is_some()
        );
    }
    assert!(!tab.read(cx, |group, _| {
        group.can_split(group.active_pane(), Direction::Row)
    }));
    assert_eq!(
        tab.update(cx, |group, cx| group.split(Direction::Row, cx)),
        None
    );
    assert_eq!(tab.panes(cx).len(), 4);
    tab.finish(cx);
}

/// Restore (lead decision L5): the snapshot holds the targets and shares in
/// the shape of docs/data-model.md, section 12, leaves out panes still
/// choosing a host, and opens every pane again.
#[gpui_kit::test]
fn snapshots_round_trip(cx: &mut TestAppContext) {
    let tab = Tab::open(cx);
    let host = save_host(&tab.state, cx);
    let one = tab.active(cx);
    let two = tab
        .update(cx, |group, cx| group.split(Direction::Row, cx))
        .unwrap();
    tab.update(cx, |group, cx| group.open_in(two, &host, cx));
    let quick = PaneTarget::Quick(QuickTarget {
        username: None,
        host: "192.0.2.30".into(),
        port: Some(2222),
    });
    let three = tab
        .update(cx, |group, cx| group.split(Direction::Column, cx))
        .unwrap();
    tab.update(cx, |group, cx| group.open_in(three, &quick, cx));
    tab.update(cx, |group, cx| {
        group.set_share(&SplitPath::default(), 0.4, cx)
    });
    // A pane still choosing is left out.
    tab.update(cx, |group, cx| group.activate(one, cx));
    let picking = tab
        .update(cx, |group, cx| group.split(Direction::Column, cx))
        .unwrap();
    let snapshot = tab
        .read(cx, |group, cx| group.snapshot(cx))
        .expect("snapshot");
    assert_eq!(snapshot.leaf_count(), 3);
    assert!(!tab.read(cx, |group, _| group.session(picking).is_some()));
    let json = serde_json::to_value(&snapshot).unwrap();
    let Some(PaneTarget::Host {
        vault_id,
        record_id,
    }) = Some(host.clone())
    else {
        unreachable!()
    };
    assert_eq!(
        json,
        json!({"direction": "row", "sizes": [0.4, 0.6], "children": [
            {"session": {"kind": "local"}},
            {"direction": "column", "sizes": [0.5, 0.5], "children": [
                {"session": {"kind": "ssh", "host": {"vault_id": vault_id, "record_id": record_id}}},
                {"session": {"kind": "ssh", "address": "192.0.2.30", "port": 2222}}
            ]}
        ]})
    );
    // No secrets or terminal data: only targets and shares.
    let text = json.to_string();
    assert!(!text.contains("password") && !text.contains("tester"));

    // Restore it in a new group.
    let back: Layout<PaneTarget> = serde_json::from_value(json).unwrap();
    let before = tab.launches();
    let restored = cx.new(|cx| PaneGroup::restore(tab.state.clone(), &back, cx));
    cx.update(|cx| {
        let group = restored.read(cx);
        assert_eq!(group.pane_count(), 3);
        assert_eq!(group.snapshot(cx).as_ref(), Some(&back));
        assert_eq!(group.active_pane(), group.pane_ids()[0]);
        assert_eq!(
            group.layout().dividers()[0].ratio,
            snapshot.dividers()[0].ratio
        );
    });
    // The sessions start without the group being drawn (a tab in the
    // background: the shell asks for it).
    assert_eq!(tab.launches(), before);
    cx.update_window(tab.handle, |_, window, cx| {
        restored.update(cx, |group, cx| group.start_pending(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab.launches(), before + 3);
    let launched: Vec<Call> = tab.record.calls().into_iter().rev().take(3).collect();
    assert!(launched.contains(&Call::Launch(host.open_request())));
    assert!(launched.contains(&Call::Launch(quick.open_request())));
    restored.update(cx, |group, cx| group.release(cx));
    tab.finish(cx);
}
