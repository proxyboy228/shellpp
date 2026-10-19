//! Headless tests of the Hosts screen alone (GPUI `TestAppContext`, no real
//! window): the screen is the window's root view, so these tests cover the
//! screen's own behavior and the requests it sends to the shell. Every test
//! runs on its own temporary profile (`TestProfile`: in-memory secret store,
//! never the OS keychain or the permanent profile). The persistence tests
//! close the core and open the same profile again with its password (a
//! restart). The minimum window tests open the whole app (`RootView`).

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
use crate::state::hosts_view::{HostsLayout, HostsSort};
use crate::state::{AppState, OpenedCore};
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

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
    });
}

impl Screen {
    /// A `width` x `height` window with the Hosts screen over a temporary
    /// profile holding `records` (a group when it has a `name`).
    fn open(cx: &mut TestAppContext, width: f32, records: &[Value]) -> (Self, Vec<Uuid>) {
        setup(cx);
        Self::open_on(cx, width, TestProfile::new().into_unlocked(), records)
    }

    /// The screen over an opened core (the same profile again after a
    /// restart); `setup` must have run once in this test.
    fn open_on(
        cx: &mut TestAppContext,
        width: f32,
        opened: OpenedCore,
        records: &[Value],
    ) -> (Self, Vec<Uuid>) {
        let state = cx.new(|cx| AppState::new(opened, cx));
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

/// Commands and drags act on the selected cards that are on screen, in
/// screen order: a search drops hidden cards from the selection, a card that
/// left the view elsewhere is not a target, and Ctrl+clicks in any order
/// place the records in the order the user sees.
#[gpui_kit::test]
fn the_selection_acts_on_cards_on_screen_in_screen_order(cx: &mut TestAppContext) {
    let (ui, ids) = Screen::open(
        cx,
        1280.,
        &[
            json!({"address": "db-a.example.com"}),
            json!({"address": "db-b.example.com"}),
            json!({"address": "web-c.example.com"}),
            json!({"name": "Lab"}),
        ],
    );
    let [a, b, c, lab] = ids[..] else {
        unreachable!()
    };
    // Ctrl+clicks in the order C, A, B: the targets are in screen order.
    ui.frame(cx, |window, cx| window.click(host_card_id(c), cx));
    for id in [a, b] {
        ui.frame(cx, |window, cx| {
            click_with(window, host_card_id(id), Modifiers::secondary_key(), cx)
        });
    }
    assert_eq!(ui.selected(cx), vec![c, a, b], "click order");
    let screen = ui.screen.clone();
    let targets = ui.frame(cx, |_, cx| screen.read(cx).menu_targets(c, cx));
    assert_eq!(targets, vec![a, b, c], "screen order");
    let dragged = ui.frame(cx, |_, cx| screen.read(cx).drag_value(a, cx));
    assert_eq!(dragged.ids, vec![a, b, c]);

    // A search that hides a selected card drops it from the selection.
    ui.type_search(cx, "db-");
    assert_eq!(ui.selected(cx), vec![a, b]);
    ui.type_search(cx, "");
    assert_eq!(ui.selected(cx), vec![a, b], "both still on screen");

    // A card that left the view elsewhere (moved into a group) is not a
    // target any more, although it is still selected.
    ui.state.update(cx, |state, cx| {
        state.move_records(vec![a], Some(lab), cx).unwrap()
    });
    cx.run_until_parked();
    assert_eq!(ui.selected(cx), vec![a, b]);
    let targets = ui.frame(cx, |_, cx| screen.read(cx).menu_targets(b, cx));
    assert_eq!(targets, vec![b]);
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

    // The tag filter (lead decision 5): the tag button opens a dropdown
    // with every tag and its host count ("prod" in both spellings).
    assert!(!ui.visible(cx, "tag-filter-menu"));
    ui.frame(cx, |window, cx| window.click("tag-filter", cx));
    assert!(ui.visible(cx, "tag-filter-menu"));
    assert!(ui.screen.read_with(cx, |screen, _| screen.tag_menu_open()));
    let options: Vec<(String, usize)> = ui.screen.read_with(cx, |screen, _| {
        screen
            .tag_options()
            .iter()
            .map(|option| (option.name.clone(), option.hosts))
            .collect()
    });
    assert_eq!(options, [("legacy".to_owned(), 1), ("prod".to_owned(), 2)]);
    // Several tags at once: every tag must match.
    ui.frame(cx, |window, cx| {
        window.click(tag_filter::tag_option_id("prod"), cx)
    });
    let mut visible = ui
        .screen
        .read_with(cx, |screen, _| screen.visible_host_ids());
    visible.sort();
    let mut expected = vec![web, mail];
    expected.sort();
    assert_eq!(visible, expected);
    assert!(
        ui.visible(cx, tag_chip_id("prod")),
        "a chip next to the button"
    );
    ui.frame(cx, |window, cx| {
        window.click(tag_filter::tag_option_id("legacy"), cx)
    });
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![mail]
    );
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.selected_tags().to_vec()),
        ["prod".to_owned(), "legacy".to_owned()]
    );
    // A click outside closes the dropdown; a chip's click turns its tag off.
    ui.frame(cx, |window, cx| window.click("hosts-content", cx));
    assert!(!ui.visible(cx, "tag-filter-menu"));
    ui.frame(cx, |window, cx| window.click(tag_chip_id("legacy"), cx));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.selected_tags().to_vec()),
        ["prod".to_owned()]
    );
    assert!(!ui.visible(cx, tag_chip_id("legacy")));
    // Clear turns every tag off.
    ui.frame(cx, |window, cx| window.click("tag-filter", cx));
    ui.frame(cx, |window, cx| window.click("tag-filter-clear", cx));
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.selected_tags().is_empty())
    );
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids())
            .len(),
        3
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

/// Bounds of the elements with these ids that are on screen.
fn bounds_of(window: &Window, ids: &[SharedString]) -> Vec<(String, Bounds<gpui::Pixels>)> {
    ids.iter()
        .filter_map(|id| {
            let found = window.try_find(id.clone())?;
            found.visible().then(|| (id.to_string(), found.bounds()))
        })
        .collect()
}

/// No two of `items` overlap (a half pixel of rounding allowed).
fn assert_disjoint(items: &[(String, Bounds<gpui::Pixels>)]) {
    for (index, (a, first)) in items.iter().enumerate() {
        for (b, second) in &items[index + 1..] {
            let overlap_x = first.right().min(second.right()) - first.left().max(second.left());
            let overlap_y = first.bottom().min(second.bottom()) - first.top().max(second.top());
            assert!(
                overlap_x <= px(0.5) || overlap_y <= px(0.5),
                "{a} {first:?} overlaps {b} {second:?}"
            );
        }
    }
}

/// Every one of `items` lies inside `outer` horizontally and vertically.
fn assert_inside(items: &[(String, Bounds<gpui::Pixels>)], outer: Bounds<gpui::Pixels>) {
    for (id, bounds) in items {
        assert!(
            bounds.left() + px(0.5) >= outer.left()
                && bounds.right() <= outer.right() + px(0.5)
                && bounds.top() + px(0.5) >= outer.top()
                && bounds.bottom() <= outer.bottom() + px(0.5),
            "{id} {bounds:?} is clipped by {outer:?}"
        );
    }
}

/// The ids of the action bar's controls and chips.
fn action_bar_ids(tags: &[&str]) -> Vec<SharedString> {
    let mut ids: Vec<SharedString> = [
        "new-host",
        "new-host-menu",
        "terminal",
        "serial",
        "view-tiles",
        "view-list",
        "tag-filter",
        "tag-chip-more",
        "sort",
    ]
    .into_iter()
    .map(SharedString::from)
    .collect();
    ids.extend(tags.iter().map(|tag| tag_chip_id(tag)));
    ids
}

const EIGHT_TAGS: [&str; 8] = [
    "prod", "staging", "linux", "windows", "critical", "eu", "us", "legacy",
];

#[gpui_kit::test]
fn tag_filter_search_counts_and_more_chip(cx: &mut TestAppContext) {
    // 700 px for the screen: every tag on does not fit next to the button.
    let (ui, _) = Screen::open(cx, 700., &hosts_with_eight_tags());
    ui.frame(cx, |window, cx| window.click("tag-filter", cx));
    // The search narrows the options (case-insensitive substring); Enter
    // turns the first match on.
    ui.frame(cx, |window, cx| {
        window.click("tag-filter-search", cx);
        window.input("IN", cx);
    });
    let menu = ui
        .screen
        .read_with(cx, |screen, _| screen.tag_menu().clone());
    let names: Vec<String> = menu.read_with(cx, |menu, cx| {
        menu.visible_options(cx)
            .into_iter()
            .map(|option| option.name)
            .collect()
    });
    assert_eq!(names, ["linux", "staging", "windows"]);
    // Space types a space (tags may hold one); the dropdown stays open.
    ui.frame(cx, |window, cx| window.press("space", cx));
    assert_eq!(menu.read_with(cx, |menu, cx| menu.search_text(cx)), "IN ");
    assert!(ui.visible(cx, "tag-filter-menu"));
    assert!(ui.visible(cx, tag_filter::tag_option_id("linux")));
    assert!(!ui.visible(cx, tag_filter::tag_option_id("prod")));
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.selected_tags().to_vec()),
        ["linux".to_owned()]
    );
    ui.frame(cx, |window, cx| {
        window.input("xyz", cx);
    });
    assert!(ui.visible(cx, "tag-filter-empty"), "no tag matches");
    ui.frame(cx, |window, cx| window.click("hosts-content", cx));

    // Every tag on: the chips that do not fit collapse into "+N", nothing
    // overlaps and nothing leaves the bar; the button shows the active state.
    ui.screen.update(cx, |screen, cx| {
        for tag in EIGHT_TAGS.iter().filter(|tag| **tag != "linux") {
            screen.toggle_tag(tag, cx);
        }
    });
    let plan = ui
        .screen
        .read_with(cx, |screen, _| screen.bar_plan())
        .unwrap();
    assert!(plan.chips < EIGHT_TAGS.len(), "{plan:?}");
    assert!(ui.visible(cx, "tag-chip-more"));
    let (screen_bounds, items) = ui.frame(cx, |window, _| {
        (
            window.find("hosts-screen").bounds(),
            bounds_of(window, &action_bar_ids(&EIGHT_TAGS)),
        )
    });
    assert!(items.iter().any(|(id, _)| id == "tag-chip-more"));
    assert_disjoint(&items);
    assert_inside(&items, screen_bounds);
    // "+N" opens the dropdown, with the focus in its search field; Esc
    // closes it.
    ui.frame(cx, |window, cx| window.click("tag-chip-more", cx));
    assert!(ui.visible(cx, "tag-filter-menu"));
    assert!(ui.screen.read_with(cx, |screen, _| screen.tag_menu_open()));
    assert!(ui.frame(cx, |window, _| {
        window.find("tag-filter-search").focused().unwrap_or(false)
    }));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert!(!ui.visible(cx, "tag-filter-menu"));
    assert!(!ui.screen.read_with(cx, |screen, _| screen.tag_menu_open()));
    ui.finish(cx);
}

/// The whole app in a window of `width` x `height` over a profile with
/// hosts carrying the eight tags and a group.
fn open_app(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    Entity<AppState>,
    AnyWindowHandle,
    Entity<crate::app::RootView>,
    Vec<Uuid>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
        crate::app::root::bind_keys(cx);
    });
    let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
    let mut records = hosts_with_eight_tags();
    records.push(json!({"name": "Production"}));
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
            size(px(width), px(height)),
        ))),
        ..Default::default()
    };
    let (handle, root) = cx
        .update(|cx| crate::app::open_window_with(state.clone(), options, cx))
        .unwrap();
    for _ in 0..3 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    (state, handle, root, ids)
}

/// At the minimum window size: the top bar, the search bar, the action bar
/// (with chips) and the first card are whole and apart.
fn check_min_window(cx: &mut TestAppContext, handle: AnyWindowHandle, tags: &[&str]) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let viewport = Bounds::new(Default::default(), window.viewport_size());
        let top: Vec<SharedString> = [
            "app-menu",
            "tab-vaults",
            "vault-switcher",
            "tab-sftp",
            "tab-new",
            "top-bar-notifications",
        ]
        .into_iter()
        .map(SharedString::from)
        .collect();
        let top = bounds_of(window, &top);
        assert_eq!(top.len(), 6, "{top:?}");
        assert_disjoint(&top);
        assert_inside(&top, viewport);

        let screen = window.find("hosts-screen").bounds();
        let main = window.find("main-area").bounds();
        assert_inside(&[("hosts-screen".to_owned(), screen)], main);
        let mut bar = bounds_of(window, &action_bar_ids(tags));
        assert!(bar.len() >= 8, "{bar:?}");
        assert!(
            tags.is_empty() || bar.iter().any(|(id, _)| id.starts_with("tag-chip-")),
            "{bar:?}"
        );
        bar.extend(bounds_of(window, &["search-field".into()]));
        assert_disjoint(&bar);
        assert_inside(&bar, screen);
        assert_inside(&bar, viewport);
        // The content keeps room for cards.
        let content = window.find("hosts-content").bounds();
        assert!(
            content.size.height >= metrics::HEIGHT_HOST_CARD * 2.,
            "{content:?}"
        );
        let cards: Vec<SharedString> = ["heading-groups", "heading-hosts"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        let cards = bounds_of(window, &cards);
        assert_inside(&cards, screen);
        if let Some(panel) = window
            .try_find("right-panel")
            .filter(|panel| panel.visible())
        {
            assert_disjoint(&[
                ("main-area".to_owned(), main),
                ("right-panel".to_owned(), panel.bounds()),
            ]);
            assert_inside(&[("right-panel".to_owned(), panel.bounds())], viewport);
        }
    })
    .unwrap();
}

/// Owner's step 1.4, lead decision 4: the window cannot get smaller than
/// 960x600, and at that size nothing is clipped or overlaps, with and
/// without the 380 px details panel, with tag chips on.
#[gpui_kit::test]
fn min_window_has_nothing_clipped_or_overlapping(cx: &mut TestAppContext) {
    let options = cx.update(|cx| crate::app::main_window_options(cx));
    assert_eq!(
        options.window_min_size,
        Some(size(px(960.), px(600.))),
        "the minimum window size"
    );
    let (state, handle, root, ids) = open_app(cx, 960., 600.);
    let screen = cx.update(|cx| {
        root.read(cx)
            .shell()
            .expect("unlocked")
            .read(cx)
            .hosts_screen()
            .clone()
    });
    // Without the details panel: one row, every label, the full search hint.
    check_min_window(cx, handle, &[]);
    let plan = screen.read_with(cx, |screen, _| screen.bar_plan()).unwrap();
    assert!(!plan.wrapped, "{plan:?}");
    let search_hint = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            screen.read(cx).search_hint_fits(window, cx)
        })
        .unwrap()
    };
    assert_eq!(search_hint(cx), (strings::SEARCH_PLACEHOLDER.into(), true));
    // Three tags on.
    screen.update(cx, |screen, cx| {
        for tag in ["prod", "staging", "critical"] {
            screen.toggle_tag(tag, cx);
        }
    });
    check_min_window(cx, handle, &["prod", "staging", "critical"]);
    screen.update(cx, |screen, cx| screen.clear_tags(cx));

    // With the details panel: the bar wraps, labels collapse as needed.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(host_card_id(ids[0]), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    let panel = cx
        .update_window(handle, |_, window, _| {
            window.try_find("right-panel").map(|panel| panel.bounds())
        })
        .unwrap()
        .expect("the details panel");
    assert_eq!(panel.size.width, metrics::WIDTH_DETAILS);
    check_min_window(cx, handle, &[]);
    // The search hint shortens instead of being cut off.
    let (hint, fits) = search_hint(cx);
    assert!(fits, "{hint}");
    screen.update(cx, |screen, cx| {
        for tag in ["prod", "staging", "critical", "legacy"] {
            screen.toggle_tag(tag, cx);
        }
    });
    check_min_window(cx, handle, &["prod", "staging", "critical", "legacy"]);
    let plan = screen.read_with(cx, |screen, _| screen.bar_plan()).unwrap();
    assert!(plan.wrapped, "{plan:?}");

    // A panel dragged to its widest keeps the main area at 360 px.
    let shell = cx.update(|cx| root.read(cx).shell().expect("unlocked").clone());
    shell.update(cx, |shell, cx| {
        shell.set_details_width(metrics::WIDTH_DETAILS_MAX, cx)
    });
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    check_min_window(cx, handle, &["prod", "staging", "critical", "legacy"]);
    let main = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("main-area").bounds()
        })
        .unwrap();
    assert_eq!(main.size.width, crate::shell::details::MAIN_AREA_MIN);
    let (hint, fits) = search_hint(cx);
    assert!(fits, "{hint}");

    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    cx.run_until_parked();
    state.update(cx, |state, _| state.shutdown());
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

/// The hosts and groups of a state, in manual order.
fn records_of(
    state: &Entity<AppState>,
    cx: &mut TestAppContext,
) -> (Vec<HostItem>, Vec<GroupItem>) {
    state.read_with(cx, |state, _| {
        (
            state.hosts().as_ref().clone(),
            state.groups().as_ref().clone(),
        )
    })
}

/// Owner's step 3.3: every Hosts operation is written at once and survives
/// a restart (the core closed, the same profile opened again with the
/// password): create and edit hosts and groups, drag to reorder and onto a
/// group, Duplicate, Move to, Copy to, Remove with Undo, Remove.
#[gpui_kit::test]
fn hosts_operations_survive_a_restart(cx: &mut TestAppContext) {
    setup(cx);
    let mut profile = TestProfile::new();
    let (ui, ids) = Screen::open_on(
        cx,
        1280.,
        profile.open_unlocked(),
        &[
            json!({"address": "a.example.com", "tags": ["prod"], "ssh": {"username": "deploy", "port": 2222}}),
            json!({"address": "b.example.com"}),
            json!({"address": "c.example.com"}),
            json!({"name": "Lab"}),
            json!({"name": "Production"}),
        ],
    );
    let [a, b, c, lab, production] = ids[..] else {
        unreachable!()
    };
    // Edit a host and a group.
    ui.state.update(cx, |state, cx| {
        let mut data = state.host(a).unwrap().data_map();
        data.insert("label".to_owned(), json!("Alpha"));
        data.insert("notes".to_owned(), json!("kept across restarts"));
        state.save_host(Some(a), data, cx).unwrap();
        let mut data = state.group(lab).unwrap().data_map();
        data.insert("name".to_owned(), json!("Lab 2"));
        state.save_group(Some(lab), data, cx).unwrap();
    });
    // Drag c onto a (Manual order): c, a, b.
    ui.frame(cx, |window, cx| {
        window.drag_to(host_card_id(c), host_card_id(a), cx)
    });
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![c, a, b]
    );
    // Drag b onto the group card: b is in Lab 2.
    ui.frame(cx, |window, cx| {
        window.drag_to(host_card_id(b), group_card_id(lab), cx)
    });
    // Duplicate a: "Alpha Copy" right after it.
    ui.frame(cx, |window, cx| window.right_click(host_card_id(a), cx));
    ui.frame(cx, |window, cx| window.click("menu-duplicate", cx));
    let copy = ui.selected(cx)[0];
    assert_ne!(copy, a);
    // Move to Production; Copy to Lab 2 (the same label).
    let screen = ui.screen.clone();
    cx.update_window(ui.handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.run_command(MenuCommand::MoveTo(Some(production)), vec![c], window, cx);
            screen.run_command(MenuCommand::CopyTo(Some(lab)), vec![a], window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let copied = ui.state.read_with(cx, |state, _| {
        state
            .hosts_in(Some(lab))
            .into_iter()
            .find(|host| host.label.as_deref() == Some("Alpha"))
            .map(|host| (host.record_id, host.data.clone()))
    });
    let (copied, copied_data) = copied.expect("the copy in Lab 2");
    assert_ne!(copied, a);
    assert_eq!(copied_data["notes"], json!("kept across restarts"));
    assert_eq!(copied_data["ssh"]["port"], json!(2222));
    assert!(
        ui.take_events()
            .contains(&HostsEvent::Toast(strings::copied_to(Some("Lab 2")).into()))
    );
    // Remove the duplicate and undo it within the 5 seconds.
    ui.take_requests();
    ui.screen.update(cx, |screen, cx| {
        screen.remove_now(vec![copy], Some("Alpha Copy".into()), cx)
    });
    let undo = ui
        .take_requests()
        .into_iter()
        .find_map(|request| match request {
            HostsRequest::ToastWithAction { action, .. } => Some(action),
            _ => None,
        })
        .expect("Undo");
    cx.executor().advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    ui.frame(cx, |window, cx| (undo.run)(window, cx));
    assert!(
        ui.state
            .read_with(cx, |state, _| state.host(copy).is_some())
    );
    // Remove b and the group Production for good (the window passes); c
    // moves up to the top level.
    ui.screen
        .update(cx, |screen, cx| screen.remove_now(vec![b], None, cx));
    ui.screen.update(cx, |screen, cx| {
        screen.remove_now(vec![production], Some("Production".into()), cx)
    });
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();

    let before = records_of(&ui.state, cx);
    assert!(before.0.iter().all(|host| host.record_id != b));
    assert_eq!(before.0.len(), 4, "a, its duplicate, c, the copy");
    assert_eq!(before.1.len(), 1, "Lab 2");
    ui.finish(cx);

    // Restart: the same profile, unlocked again with its password.
    let (ui, _) = Screen::open_on(cx, 1280., profile.open_unlocked(), &[]);
    let after = records_of(&ui.state, cx);
    assert_eq!(after, before, "every host and group, field and order");
    ui.state.read_with(cx, |state, _| {
        assert_eq!(state.host(a).unwrap().label.as_deref(), Some("Alpha"));
        assert_eq!(state.group(lab).unwrap().name, "Lab 2");
        assert!(
            state.group(production).is_none(),
            "the group removal is final"
        );
        assert_eq!(state.host(c).unwrap().group_id, None, "its host moved up");
        assert_eq!(state.host(copied).unwrap().group_id, Some(lab));
        assert!(state.host(b).is_none(), "the removal is final");
        assert_eq!(
            state.host(copy).unwrap().label.as_deref(),
            Some("Alpha Copy")
        );
    });
    // The manual order of the top level: a, its duplicate, then c (moved up
    // from the removed group, at the end).
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.visible_host_ids()),
        vec![a, copy, c]
    );
    ui.finish(cx);
    drop(profile);
}

/// UI 2.4: Undo works for 5 seconds and brings a removed group back with
/// its children; after the window the removal is final, also after a
/// restart.
#[gpui_kit::test]
fn undo_window_of_remove(cx: &mut TestAppContext) {
    setup(cx);
    let mut profile = TestProfile::new();
    let (ui, ids) = Screen::open_on(
        cx,
        1280.,
        profile.open_unlocked(),
        &[
            json!({"name": "Lab"}),
            json!({"address": "x.example.com"}),
            json!({"address": "y.example.com"}),
        ],
    );
    let [lab, x, y] = ids[..] else { unreachable!() };
    ui.state
        .update(cx, |state, cx| state.move_records(vec![x], Some(lab), cx))
        .unwrap();
    let before = records_of(&ui.state, cx);

    // Remove the non-empty group: x moves up; Undo puts it back inside.
    ui.state
        .update(cx, |state, cx| state.delete_records(vec![lab], cx))
        .unwrap();
    assert_eq!(
        ui.state
            .read_with(cx, |state, _| state.host(x).unwrap().group_id),
        None
    );
    assert!(
        ui.state
            .read_with(cx, |state, _| state.can_undo_removal(&[lab]))
    );
    ui.state
        .update(cx, |state, cx| state.restore_records(vec![lab], cx))
        .unwrap();
    let fields = |groups: &[GroupItem]| -> Vec<(Uuid, Value, Option<Uuid>)> {
        groups
            .iter()
            .map(|group| (group.record_id, group.data.clone(), group.parent_id))
            .collect()
    };
    let restored = records_of(&ui.state, cx);
    assert_eq!(
        fields(&restored.1),
        fields(&before.1),
        "the group, same id and fields"
    );
    assert_eq!(
        ui.state
            .read_with(cx, |state, _| state.host(x).unwrap().group_id),
        Some(lab),
        "its host is back inside"
    );

    // After 5 seconds Undo no longer works.
    ui.state
        .update(cx, |state, cx| state.delete_records(vec![y], cx))
        .unwrap();
    cx.executor().advance_clock(crate::state::undo::UNDO_WINDOW);
    cx.run_until_parked();
    assert!(
        !ui.state
            .read_with(cx, |state, _| state.can_undo_removal(&[y]))
    );
    let late = ui
        .state
        .update(cx, |state, cx| state.restore_records(vec![y], cx));
    assert!(matches!(late, Err(shellpp_core::CoreError::NotFound)));
    ui.finish(cx);

    let (ui, _) = Screen::open_on(cx, 1280., profile.open_unlocked(), &[]);
    ui.state.read_with(cx, |state, _| {
        assert!(state.host(y).is_none());
        assert_eq!(state.host(x).unwrap().group_id, Some(lab));
        assert!(
            !state.can_undo_removal(&[y]),
            "nothing to undo after a restart"
        );
    });
    ui.finish(cx);
    drop(profile);
}
