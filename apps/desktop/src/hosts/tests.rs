//! Headless tests of the Hosts screen alone (GPUI `TestAppContext`, no real
//! window): the screen is the window's root view, so these tests cover the
//! screen's own behavior and the requests it sends to the shell. Every test
//! runs on its own temporary profile (`TestProfile`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, InputEvent as _, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::*;
use crate::state::AppState;
use crate::state::hosts_view::{HostsLayout, HostsSort};
use crate::testing::TestProfile;
use crate::theme::{self, Appearance, metrics};

/// What the screen emitted, in order.
#[derive(Default)]
struct Log {
    events: Vec<HostsEvent>,
    requests: Vec<HostsRequest>,
}

struct Screen {
    state: Entity<AppState>,
    screen: Entity<HostsScreen>,
    handle: AnyWindowHandle,
    log: Rc<RefCell<Log>>,
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

impl Screen {
    /// A `width` x `height` window with the Hosts screen over a temporary
    /// profile holding `records` (a group when it has a `name`).
    fn open(cx: &mut TestAppContext, width: f32, records: &[Value]) -> (Self, Vec<Uuid>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let ids = state.update(cx, |state, cx| {
            records
                .iter()
                .map(|record| {
                    let data = map(record.clone());
                    if data.contains_key("address") {
                        state.save_host(None, data, cx).unwrap().record_id
                    } else {
                        state.save_group(None, data, cx).unwrap().record_id
                    }
                })
                .collect::<Vec<_>>()
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(800.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| HostsScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let log = Rc::new(RefCell::new(Log::default()));
        cx.update(|cx| {
            let events = log.clone();
            cx.subscribe(&screen, move |_, event: &HostsEvent, _| {
                events.borrow_mut().events.push(event.clone())
            })
            .detach();
            let requests = log.clone();
            cx.subscribe(&screen, move |_, request: &HostsRequest, _| {
                requests.borrow_mut().requests.push(request.clone())
            })
            .detach();
        });
        let this = Self {
            state,
            screen,
            handle,
            log,
        };
        // The second frame lays the grid out with the measured width.
        this.frame(cx, |_, _| {});
        this.frame(cx, |_, _| {});
        (this, ids)
    }

    fn frame<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Window, &mut gpui::App) -> R,
    ) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn type_search(&self, cx: &mut TestAppContext, text: &str) {
        self.frame(cx, |window, cx| {
            window.click("search", cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            if !text.is_empty() {
                window.input(text, cx);
            }
        });
    }

    fn selected(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.state
            .read_with(cx, |state, _| state.selection().ids().to_vec())
    }

    fn take_requests(&self) -> Vec<HostsRequest> {
        std::mem::take(&mut self.log.borrow_mut().requests)
    }

    fn take_events(&self) -> Vec<HostsEvent> {
        std::mem::take(&mut self.log.borrow_mut().events)
    }

    fn sessions(&self) -> Vec<SessionRequest> {
        self.log
            .borrow()
            .requests
            .iter()
            .filter_map(|request| match request {
                HostsRequest::OpenSession(session) => Some(session.clone()),
                _ => None,
            })
            .collect()
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// A left click on an element with modifiers held (Ctrl, Shift).
fn click_with(
    window: &mut Window,
    id: impl Into<ElementId>,
    modifiers: Modifiers,
    cx: &mut gpui::App,
) {
    window.render_frame(cx);
    let position = window.find(id).bounds().center();
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
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
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn hosts_with_group() -> Vec<Value> {
    vec![
        json!({"address": "web1.example.com", "label": "Web front", "tags": ["prod"], "ssh": {"username": "deploy"}}),
        json!({"address": "192.0.2.10", "ssh": {"username": "root"}}),
        json!({"address": "mail.example.org", "label": "Mail", "tags": ["legacy", "Prod"]}),
        json!({"name": "Databases"}),
    ]
}

#[gpui_kit::test]
fn search_filters_and_drives_connect(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(cx, 1280., &hosts_with_group());
    let [web, ip, mail, databases] = ids[..] else {
        unreachable!()
    };
    // No autofocus; Connect disabled.
    ui.frame(cx, |window, _| {
        assert_eq!(window.find("search").focused(), Some(false));
    });
    assert!(
        !ui.screen
            .read_with(cx, |screen, _| screen.connect_enabled())
    );
    assert!(ui.visible(cx, group_card_id(databases)));

    ui.type_search(cx, "deploy");
    let screen = ui.screen.clone();
    assert!(ui.frame(cx, |window, cx| screen.read(cx).search_focused(window, cx)));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![web]
    );
    assert!(!ui.visible(cx, group_card_id(databases)));
    assert!(
        !ui.screen
            .read_with(cx, |screen, _| screen.connect_enabled())
    );

    ui.type_search(cx, "datab");
    assert!(ui.visible(cx, group_card_id(databases)));
    assert!(!ui.visible(cx, host_card_id(ip)));

    // A recognized address enables Connect and tells the shell.
    ui.take_requests();
    ui.type_search(cx, "admin@db");
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.connect_enabled())
    );
    assert!(
        ui.take_requests()
            .iter()
            .any(|request| matches!(request, HostsRequest::SearchAddressChanged(true)))
    );
    ui.type_search(cx, "web server");
    assert!(
        !ui.screen
            .read_with(cx, |screen, _| screen.connect_enabled())
    );
    assert!(
        ui.take_requests()
            .iter()
            .any(|request| matches!(request, HostsRequest::SearchAddressChanged(false)))
    );

    // Exactly a saved host's label: Connect connects to that host.
    ui.type_search(cx, "Mail");
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.search_target().clone()),
        SearchTarget::Saved(mail)
    );
    ui.frame(cx, |window, cx| window.click("quick-connect", cx));
    let sessions = ui.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, Some(mail));
    assert_eq!(sessions[0].kind, SessionKind::Ssh);
    ui.finish(cx);
}

#[gpui_kit::test]
fn quick_connect_offers_to_save_the_host(cx: &mut TestAppContext) {
    let (ui, _) = Screen::open(cx, 1280., &[json!({"address": "web1.example.com"})]);
    ui.type_search(cx, "prod");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert!(ui.sessions().is_empty(), "a search word does not connect");

    ui.type_search(cx, "ssh root@192.0.2.77 -p 2222");
    ui.take_requests();
    ui.frame(cx, |window, cx| window.press("enter", cx));
    let requests = ui.take_requests();
    let session = requests
        .iter()
        .find_map(|request| match request {
            HostsRequest::OpenSession(session) => Some(session.clone()),
            _ => None,
        })
        .expect("a session tab");
    assert_eq!(session.host, None);
    assert_eq!(session.title.as_ref(), "root@192.0.2.77");
    assert_eq!(
        session.target,
        Some(QuickTarget {
            username: Some("root".into()),
            host: "192.0.2.77".into(),
            port: Some(2222),
        })
    );
    let action = requests
        .iter()
        .find_map(|request| match request {
            HostsRequest::ToastWithAction { message, action } => {
                assert_eq!(message.as_ref(), strings::SAVE_HOST_OFFER);
                Some(action.clone())
            }
            _ => None,
        })
        .expect("the save-host offer");
    assert_eq!(ui.state.read_with(cx, |state, _| state.hosts().len()), 1);

    // "Save" creates the host with the user and the port, selects it and
    // opens its panel.
    ui.frame(cx, |window, cx| (action.run)(window, cx));
    let hosts = ui.state.read_with(cx, |state, _| state.hosts());
    assert_eq!(hosts.len(), 2);
    let saved = hosts
        .iter()
        .find(|host| host.address == "192.0.2.77")
        .unwrap();
    assert_eq!(saved.username.as_deref(), Some("root"));
    assert_eq!(saved.port, Some(2222));
    assert_eq!(ui.selected(cx), vec![saved.record_id]);
    assert!(
        ui.take_events()
            .contains(&HostsEvent::OpenHost(saved.record_id))
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn clicks_select_open_and_connect(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(
        cx,
        1280.,
        &[
            json!({"address": "a.example.com"}),
            json!({"address": "b.example.com"}),
            json!({"address": "c.example.com"}),
            json!({"address": "d.example.com"}),
        ],
    );
    let [a, b, c, d] = ids[..] else {
        unreachable!()
    };
    ui.frame(cx, |window, cx| window.click(host_card_id(a), cx));
    assert_eq!(ui.selected(cx), vec![a]);
    assert_eq!(ui.take_events(), vec![HostsEvent::OpenHost(a)]);
    assert!(ui.sessions().is_empty());

    // Ctrl+click toggles, Shift+click selects a range in screen order.
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(c), Modifiers::secondary_key(), cx)
    });
    assert_eq!(ui.selected(cx), vec![a, c]);
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(a), Modifiers::secondary_key(), cx)
    });
    assert_eq!(ui.selected(cx), vec![c]);
    ui.frame(cx, |window, cx| window.click(host_card_id(b), cx));
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(d), Modifiers::shift(), cx)
    });
    assert_eq!(ui.selected(cx), vec![b, c, d]);
    // Only plain clicks open the panel.
    assert_eq!(
        ui.take_events(),
        vec![HostsEvent::OpenHost(b)],
        "Ctrl and Shift clicks only select"
    );

    // Double click: a session tab for the host.
    ui.frame(cx, |window, cx| window.double_click(host_card_id(d), cx));
    let sessions = ui.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, Some(d));
    assert!(!sessions[0].new_window);
    assert_eq!(ui.selected(cx), vec![d]);
    ui.finish(cx);
}

#[gpui_kit::test]
fn keys_connect_edit_and_move_the_selection(cx: &mut TestAppContext) {
    let records: Vec<Value> = (0..6)
        .map(|n| json!({"address": format!("h{n}.example.com")}))
        .collect();
    // 1280 - 220 (the sidebar the shell adds) = 1060 px for the screen.
    let (ui, ids) = Screen::open(cx, 1060., &records);
    let columns = ui.screen.read_with(cx, |screen, _| screen.columns());
    assert_eq!(columns, 3, "1280 px window: 3 columns");

    ui.frame(cx, |window, cx| window.click(host_card_id(ids[0]), cx));
    ui.take_events();
    ui.frame(cx, |window, cx| window.press("e", cx));
    assert!(matches!(
        ui.take_requests().last(),
        Some(HostsRequest::EditHost(id)) if *id == ids[0]
    ));
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(ui.sessions().len(), 1);

    ui.frame(cx, |window, cx| window.press("right", cx));
    assert_eq!(ui.selected(cx), vec![ids[1]]);
    ui.frame(cx, |window, cx| window.press("down", cx));
    assert_eq!(ui.selected(cx), vec![ids[1 + columns]]);
    ui.frame(cx, |window, cx| window.press("left", cx));
    assert_eq!(ui.selected(cx), vec![ids[columns]]);
    ui.frame(cx, |window, cx| window.press("up", cx));
    assert_eq!(ui.selected(cx), vec![ids[0]]);
    // The panel follows the keyboard selection.
    assert!(
        ui.take_events()
            .contains(&HostsEvent::OpenHost(ids[columns]))
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn context_menu_runs_the_card_actions(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(cx, 1280., &hosts_with_group());
    let [web, ip, _, databases] = ids[..] else {
        unreachable!()
    };
    // The model: the order of UI 2.4.
    let entries = ui
        .screen
        .read_with(cx, |screen, cx| screen.context_menu_for(web, cx));
    assert_eq!(
        menu::entry_ids(&entries),
        [
            "menu-quick-connect",
            "menu-connect",
            "menu-add-to-workspace",
            "menu-edit-host",
            "menu-collaborate",
            "menu-move-to",
            "menu-copy-to",
            "menu-duplicate",
            "menu-copy-link",
            "menu-remove",
        ]
    );

    // Right click selects the card and opens the menu; Duplicate.
    ui.frame(cx, |window, cx| window.right_click(host_card_id(web), cx));
    assert_eq!(ui.selected(cx), vec![web]);
    assert!(ui.visible(cx, "menu-quick-connect"));
    assert!(ui.visible(cx, "menu-remove"));
    ui.frame(cx, |window, cx| window.click("menu-duplicate", cx));
    let hosts = ui.state.read_with(cx, |state, _| state.hosts());
    assert_eq!(hosts.len(), 4);
    let copy = hosts
        .iter()
        .find(|host| host.label.as_deref() == Some("Web front Copy"))
        .expect("the copy");
    assert_eq!(ui.selected(cx), vec![copy.record_id]);
    let copy = copy.record_id;

    // Copy Link: the app URL in the clipboard and a toast.
    ui.frame(cx, |window, cx| window.right_click(host_card_id(web), cx));
    ui.frame(cx, |window, cx| window.click("menu-copy-link", cx));
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard, Some(format!("shellpp://host/{web}")));
    assert!(
        ui.take_events()
            .contains(&HostsEvent::Toast(strings::LINK_COPIED.into()))
    );

    // Move to ▸ Databases through the command (submenus open on hover).
    let screen = ui.screen.clone();
    cx.update_window(ui.handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.run_command(MenuCommand::MoveTo(Some(databases)), vec![ip], window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let moved = ui
        .state
        .read_with(cx, |state, _| state.host(ip).unwrap().group_id);
    assert_eq!(moved, Some(databases));

    // Remove asks first, removes, then offers Undo.
    ui.frame(cx, |window, cx| window.right_click(host_card_id(copy), cx));
    ui.frame(cx, |window, cx| window.click("menu-remove", cx));
    assert!(ui.visible(cx, "remove-dialog"));
    assert!(
        ui.state
            .read_with(cx, |state, _| state.host(copy).is_some())
    );
    ui.take_requests();
    ui.frame(cx, |window, cx| window.click("remove-confirm", cx));
    assert!(
        ui.state
            .read_with(cx, |state, _| state.host(copy).is_none())
    );
    let undo = ui
        .take_requests()
        .into_iter()
        .find_map(|request| match request {
            HostsRequest::ToastWithAction { action, .. } => Some(action),
            _ => None,
        })
        .expect("Undo");
    assert_eq!(undo.label.as_ref(), strings::UNDO);
    ui.frame(cx, |window, cx| (undo.run)(window, cx));
    assert!(
        ui.state
            .read_with(cx, |state, _| state.host(copy).is_some())
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn groups_are_folders_with_breadcrumbs(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(cx, 1280., &hosts_with_group());
    let databases = ids[3];
    let db = ui.state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                map(json!({"address": "db1.example.com", "group_id": databases})),
                cx,
            )
            .unwrap()
            .record_id
    });
    assert_eq!(
        ui.state
            .read_with(cx, |state, _| state.host(db).unwrap().group_id),
        Some(databases)
    );
    // The top level: section headings, no breadcrumbs, the group card counts
    // its hosts.
    assert!(ui.visible(cx, "heading-groups"));
    assert!(ui.visible(cx, "heading-hosts"));
    assert!(!ui.visible(cx, "breadcrumbs"));
    assert!(!ui.visible(cx, host_card_id(db)));

    ui.frame(cx, |window, cx| window.click(group_card_id(databases), cx));
    assert_eq!(
        ui.state.read_with(cx, |state, _| state.location()),
        Some(databases)
    );
    assert!(ui.visible(cx, "breadcrumbs"));
    assert!(ui.visible(cx, host_card_id(db)));
    assert!(!ui.visible(cx, host_card_id(ids[0])));

    ui.frame(cx, |window, cx| window.click("crumb-root", cx));
    assert_eq!(ui.state.read_with(cx, |state, _| state.location()), None);
    assert!(!ui.visible(cx, "breadcrumbs"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn grid_columns_follow_the_width(cx: &mut TestAppContext) {
    let records: Vec<Value> = (0..12)
        .map(|n| json!({"address": format!("h{n}.example.com")}))
        .collect();
    // 1920 - 220 (the sidebar the shell adds) = 1700 px for the screen.
    let (ui, ids) = Screen::open(cx, 1700., &records);
    let (columns, width) = ui
        .screen
        .read_with(cx, |screen, _| (screen.columns(), screen.card_width()));
    assert_eq!(columns, 5, "1920 px window: 5 columns");
    assert!((240. ..=360.).contains(&width), "{width}");
    let card = ui.frame(cx, |window, _| window.find(host_card_id(ids[0])).bounds());
    assert_eq!(card.size.height, metrics::HEIGHT_HOST_CARD);
    assert!((card.size.width.as_f32() - width).abs() < 0.5);
    let next = ui.frame(cx, |window, _| window.find(host_card_id(ids[1])).bounds());
    let gap = next.origin.x - (card.origin.x + card.size.width);
    assert!((gap.as_f32() - metrics::HOST_CARD_GAP.as_f32()).abs() < 0.5);
    ui.finish(cx);
}

#[gpui_kit::test]
fn view_sort_and_tag_filter(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(cx, 1280., &hosts_with_group());
    let [web, ip, mail, _] = ids[..] else {
        unreachable!()
    };
    // The list layout, remembered in the settings.
    ui.frame(cx, |window, cx| window.click("view-list", cx));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.layout()),
        HostsLayout::List
    );
    assert_eq!(
        ui.state.read_with(cx, |state, _| state.hosts_layout()),
        HostsLayout::List
    );
    let row = ui.frame(cx, |window, _| window.find(host_card_id(web)).bounds());
    assert_eq!(row.size.height, metrics::HEIGHT_ROW);
    ui.frame(cx, |window, cx| window.click("view-tiles", cx));

    // Sort by name: "192.0.2.10", "Mail", "Web front".
    ui.screen
        .update(cx, |screen, cx| screen.set_sort(HostsSort::Name, cx));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![ip, mail, web]
    );
    assert_eq!(
        ui.state.read_with(cx, |state, _| state.hosts_sort()),
        HostsSort::Name
    );

    // Tag chips: "prod" matches both spellings.
    ui.frame(cx, |window, cx| window.click(tag_chip_id("prod"), cx));
    let mut visible = ui
        .screen
        .read_with(cx, |screen, _| screen.visible_host_ids());
    visible.sort();
    let mut expected = vec![web, mail];
    expected.sort();
    assert_eq!(visible, expected);
    ui.frame(cx, |window, cx| window.click(tag_chip_id("legacy"), cx));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![mail]
    );
    ui.finish(cx);
}

/// Eight tags, as in the demo profile.
fn hosts_with_eight_tags() -> Vec<Value> {
    let tags = [
        "prod", "staging", "linux", "windows", "critical", "eu", "us", "legacy",
    ];
    tags.iter()
        .enumerate()
        .map(|(n, tag)| json!({"address": format!("h{n}.example.com"), "tags": [tag]}))
        .collect()
}

#[gpui_kit::test]
fn tag_filter_uses_the_free_room(cx: &mut TestAppContext) {
    // 1920 px window (1700 px screen): every chip is whole, none clipped.
    let (ui, _) = Screen::open(cx, 1700., &hosts_with_eight_tags());
    let (strip, last) = ui.frame(cx, |window, _| {
        (
            window.find("tag-filter").bounds(),
            window.find(tag_chip_id("windows")).bounds(),
        )
    });
    assert!(
        last.right() <= strip.right() + px(0.5),
        "last chip {last:?} clipped by the strip {strip:?}"
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn tag_filter_scrolls_when_the_room_runs_out(cx: &mut TestAppContext) {
    // A narrow screen: the strip shrinks and scrolls; the sort button stays
    // inside the action bar.
    let (ui, _) = Screen::open(cx, 700., &hosts_with_eight_tags());
    let (bar, strip, sort) = ui.frame(cx, |window, _| {
        (
            window.find("action-bar").bounds(),
            window.find("tag-filter").bounds(),
            window.find("sort").bounds(),
        )
    });
    assert!(strip.right() <= sort.left() + px(0.5), "{strip:?} {sort:?}");
    assert!(sort.right() <= bar.right() + px(0.5), "{sort:?} {bar:?}");
    ui.finish(cx);
}

#[gpui_kit::test]
fn drag_moves_into_a_group_and_reorders(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(
        cx,
        1280.,
        &[
            json!({"address": "a.example.com"}),
            json!({"address": "b.example.com"}),
            json!({"address": "c.example.com"}),
            json!({"name": "Lab"}),
        ],
    );
    let [a, b, c, lab] = ids[..] else {
        unreachable!()
    };
    // Manual order: drop c on a puts it before a.
    ui.frame(cx, |window, cx| {
        window.drag_to(host_card_id(c), host_card_id(a), cx)
    });
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![c, a, b]
    );
    // Drop on a group card: moved into the group.
    ui.frame(cx, |window, cx| {
        window.drag_to(host_card_id(b), group_card_id(lab), cx)
    });
    assert_eq!(
        ui.state
            .read_with(cx, |state, _| state.host(b).unwrap().group_id),
        Some(lab)
    );
    // Another sort mode: no reordering.
    ui.screen
        .update(cx, |screen, cx| screen.set_sort(HostsSort::Name, cx));
    ui.frame(cx, |window, cx| {
        window.drag_to(host_card_id(c), host_card_id(a), cx)
    });
    ui.screen
        .update(cx, |screen, cx| screen.set_sort(HostsSort::Manual, cx));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![c, a]
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn empty_state_and_action_bar(cx: &mut TestAppContext) {
    let (ui, _) = Screen::open(cx, 1280., &[]);
    assert!(ui.visible(cx, "hosts-empty"));
    assert!(ui.visible(cx, "create-host"));
    assert!(ui.visible(cx, "import-ssh-config"));
    ui.frame(cx, |window, cx| window.click("create-host", cx));
    assert_eq!(ui.take_events(), vec![HostsEvent::NewHost]);
    ui.frame(cx, |window, cx| window.click("new-host", cx));
    assert_eq!(ui.take_events(), vec![HostsEvent::NewHost]);

    // Terminal and Serial open placeholder session tabs.
    ui.frame(cx, |window, cx| window.click("terminal", cx));
    ui.frame(cx, |window, cx| window.click("serial", cx));
    let kinds: Vec<SessionKind> = ui.sessions().iter().map(|session| session.kind).collect();
    assert_eq!(kinds, vec![SessionKind::Terminal, SessionKind::Serial]);

    // Import (empty state) opens the placeholder dialog.
    ui.frame(cx, |window, cx| window.click("import-ssh-config", cx));
    assert!(ui.visible(cx, "import-dialog"));
    ui.frame(cx, |window, cx| window.click("placeholder-close", cx));
    assert!(!ui.visible(cx, "import-dialog"));

    // New host ▸ New Group.
    ui.frame(cx, |window, cx| window.click("new-host-menu", cx));
    ui.frame(cx, |window, cx| window.click("new-group", cx));
    assert_eq!(ui.take_events(), vec![HostsEvent::NewGroup]);
    ui.finish(cx);
}

#[gpui_kit::test]
fn quick_connect_from_the_button(cx: &mut TestAppContext) {
    // The search Connect button quick-connects like Enter.
    let (ui, _) = Screen::open(cx, 1280., &[]);
    ui.type_search(cx, "10.0.0.1");
    ui.frame(cx, |window, cx| window.click("quick-connect", cx));
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    assert_eq!(ui.sessions().len(), 1);
    ui.finish(cx);
}

#[test]
fn sections_navigation() {
    use layout::Arrow;
    // 5 groups, 4 hosts, 4 columns.
    assert_eq!(sections_target(5, 4, Some(4), 4, Arrow::Down), Some(5));
    assert_eq!(sections_target(5, 4, Some(1), 4, Arrow::Down), Some(6));
    assert_eq!(sections_target(5, 4, Some(4), 4, Arrow::Right), Some(5));
    assert_eq!(sections_target(5, 4, Some(5), 4, Arrow::Left), Some(4));
    assert_eq!(sections_target(5, 4, Some(6), 4, Arrow::Up), Some(4));
    assert_eq!(sections_target(5, 4, Some(5), 4, Arrow::Up), Some(4));
    assert_eq!(sections_target(0, 4, None, 4, Arrow::Up), Some(0));
    assert_eq!(sections_target(0, 0, None, 4, Arrow::Up), None);
    assert_eq!(sections_target(5, 0, Some(4), 4, Arrow::Down), Some(4));
}

#[test]
fn exact_match_prefers_the_exact_spelling() {
    use crate::state::HostItem;
    use shellpp_core::service::dto::TagColor;
    let host = |label: &str, address: &str| HostItem {
        record_id: Uuid::now_v7(),
        vault_id: Uuid::nil(),
        revision: 1,
        label: Some(label.to_owned()),
        address: address.to_owned(),
        tags: Vec::new(),
        color: TagColor::default(),
        username: None,
        port: None,
        os_family: None,
        group_id: None,
        path: Vec::new(),
        created_at: String::new(),
        modified_at: String::new(),
        data: Value::Null,
        secrets_set: Vec::new(),
    };
    let hosts = vec![host("web", "a.lan"), host("Web", "b.lan")];
    assert_eq!(exact_match(&hosts, "Web"), Some(hosts[1].record_id));
    assert_eq!(exact_match(&hosts, "WEB"), Some(hosts[0].record_id));
    assert_eq!(exact_match(&hosts, "b.lan"), Some(hosts[1].record_id));
    assert_eq!(exact_match(&hosts, "c.lan"), None);
}
