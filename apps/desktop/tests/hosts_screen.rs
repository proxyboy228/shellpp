//! Headless UI tests of the Hosts screen (owner's step 5; UI 2.1-2.4):
//! search filtering, address recognition and the Connect button (enabled
//! state and accent fill), single and double click on a card, Ctrl/Shift
//! selection, Enter and E, the card context menu (composition and order),
//! and the toast position. GPUI `TestAppContext`, no real window; every test
//! runs on its own temporary profile.

mod common;

use common::{Ui, accent_filled, accent_fills, bounds, click_with, ctrl, numbered, shift, visible};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, TestAppContext, Window};
use serde_json::json;
use shellpp_desktop::hosts::{group_card_id, host_card_id};
use uuid::Uuid;

/// Element ids used by these tests (one place to keep them in step with
/// the views).
mod ids {
    pub const SEARCH: &str = "search";
    pub const QUICK_CONNECT: &str = "quick-connect";
    pub const HOST_PANEL: &str = "host-panel";
    pub const HOST_ADDRESS: &str = "host-address";
    pub const HOST_CONNECT: &str = "host-connect";
    pub const RIGHT_PANEL: &str = "right-panel";
    pub const MAIN_AREA: &str = "main-area";
    pub const POPUP_MENU: &str = "popup-menu";
    /// Session tabs in the top bar: `session-tab-<n>`.
    pub const SESSION_TAB_PREFIX: &str = "session-tab-";
    /// Toasts at the bottom of the main area: `toast-<n>`.
    pub const TOAST_PREFIX: &str = "toast-";
}

fn sample() -> Vec<serde_json::Value> {
    vec![
        json!({"address": "web1.example.com", "label": "Web front", "tags": ["prod"], "ssh": {"username": "deploy"}}),
        json!({"address": "192.0.2.10", "ssh": {"username": "root"}}),
        json!({"address": "mail.example.org", "label": "Mail", "tags": ["legacy"]}),
        json!({"name": "Databases"}),
        json!({"name": "Office"}),
    ]
}

/// Replaces the search text (click, select all, delete, type).
fn type_search(ui: &Ui, cx: &mut TestAppContext, text: &str) {
    ui.frame(cx, |window, cx| {
        window.click(ids::SEARCH, cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        if !text.is_empty() {
            window.input(text, cx);
        }
    });
    ui.settle(cx);
}

fn session_tabs(window: &Window) -> usize {
    numbered(window, ids::SESSION_TAB_PREFIX).len()
}

fn toasts(window: &Window) -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
    numbered(window, ids::TOAST_PREFIX)
        .into_iter()
        .map(|(_, snapshot)| snapshot.bounds())
        .collect()
}

/// UI 2.1: typing filters hosts by label, address, tags and username, and
/// groups by name, case-insensitively; clearing the search restores all.
/// No autofocus in the search bar.
#[gpui_kit::test]
fn search_filters_hosts_and_groups(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &sample());
    let [web, ip, mail, databases, office] = ids[..] else {
        unreachable!()
    };
    let hosts = [web, ip, mail];

    ui.frame(cx, |window, _| {
        assert_eq!(window.find(ids::SEARCH).focused(), Some(false));
        for id in hosts {
            assert!(visible(window, host_card_id(id)), "{id}");
        }
        assert!(visible(window, group_card_id(databases)));
        assert!(visible(window, group_card_id(office)));
    });

    let cases: [(&str, &[Uuid]); 6] = [
        ("web", &[web]),     // label and address
        ("192.0.2", &[ip]),  // address
        ("LEGACY", &[mail]), // tag, any case
        ("deploy", &[web]),  // username
        ("example", &[web, mail]),
        ("no-such-host", &[]),
    ];
    for (query, expected) in cases {
        type_search(&ui, cx, query);
        ui.frame(cx, |window, _| {
            for id in hosts {
                assert_eq!(
                    visible(window, host_card_id(id)),
                    expected.contains(&id),
                    "query {query:?}, host {id}"
                );
            }
        });
    }

    // Groups match by name.
    type_search(&ui, cx, "datab");
    ui.frame(cx, |window, _| {
        assert!(visible(window, group_card_id(databases)));
        assert!(!visible(window, group_card_id(office)));
        for id in hosts {
            assert!(!visible(window, host_card_id(id)));
        }
    });

    type_search(&ui, cx, "");
    ui.frame(cx, |window, _| {
        for id in hosts {
            assert!(visible(window, host_card_id(id)));
        }
        assert!(visible(window, group_card_id(office)));
    });
    ui.finish(cx);
}

/// UI 2.1 and owner's audit item 2: Connect in the search bar is disabled
/// (and not accent-filled) unless the text is an address: `host` (only with
/// a dot or colon, an IP or `localhost`), `user@host`, `user@host:port`,
/// `ssh user@host -p port`, or exactly a saved host's label or address.
#[gpui_kit::test]
fn connect_follows_address_recognition(cx: &mut TestAppContext) {
    let (ui, _) = Ui::open(
        cx,
        &[json!({"address": "web1.example.com", "label": "Web"})],
    );
    let check = |ui: &Ui, cx: &mut TestAppContext, text: &str, address: bool| {
        type_search(ui, cx, text);
        ui.frame(cx, |window, cx| {
            let connect = window.find(ids::QUICK_CONNECT);
            if let Some(disabled) = connect.disabled() {
                assert_eq!(disabled, !address, "{text:?}: disabled flag");
            }
            assert_eq!(
                accent_filled(window, ids::QUICK_CONNECT, cx),
                address,
                "{text:?}: accent fill"
            );
        });
    };

    check(&ui, cx, "", false);
    for text in ["prod", "web server", "ssh", "ssh -p 2222", "Webb"] {
        check(&ui, cx, text, false);
    }
    for text in [
        "db.example.com",
        "10.0.0.1",
        "localhost",
        "prod.lan",
        "admin@db",
        "root@192.0.2.5:2222",
        "ssh root@192.0.2.5 -p 2222",
        // Exactly a saved host's label: connects to that host.
        "Web",
    ] {
        check(&ui, cx, text, true);
    }
    ui.finish(cx);
}

/// UI 2.1, 2.4: Enter in the search bar with an address quick-connects
/// (a session tab) without saving a host; a non-address does nothing. The
/// offer to save the host waits until the session has connected (UI 2.1:
/// "after a successful quick connect"; session_ui's
/// `a_quick_connect_offers_to_save_the_host_once_connected`), so it is not
/// shown on Enter.
#[gpui_kit::test]
fn quick_connect_opens_a_session_without_saving(cx: &mut TestAppContext) {
    let (ui, _) = Ui::open(cx, &[json!({"address": "web1.example.com"})]);
    let before = ui.hosts(cx).len();

    type_search(&ui, cx, "prod");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| assert_eq!(session_tabs(window), 0));

    type_search(&ui, cx, "root@192.0.2.77");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        assert_eq!(session_tabs(window), 1);
        assert!(
            toasts(window).is_empty(),
            "no save-host offer before connecting"
        );
    });
    assert_eq!(ui.hosts(cx).len(), before, "nothing is saved");
    ui.finish(cx);
}

/// Lead decision 3: one accent-filled button on screen. With the host panel
/// open, its Connect is the accent button; while the search text is an
/// address, the search Connect is, and the panel's Connect is neutral.
#[gpui_kit::test]
fn only_one_connect_is_accent_filled(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open_sized(cx, 1920., 1200., &sample());
    let web = ids[0];
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        assert!(visible(window, ids::HOST_PANEL));
        assert!(accent_filled(window, ids::HOST_CONNECT, cx));
        assert!(!accent_filled(window, ids::QUICK_CONNECT, cx));
        assert_eq!(accent_fills(window, 40., 24., cx), 1, "one accent button");
    });

    type_search(&ui, cx, "admin@192.0.2.44");
    ui.frame(cx, |window, cx| {
        assert!(accent_filled(window, ids::QUICK_CONNECT, cx));
        assert!(!accent_filled(window, ids::HOST_CONNECT, cx));
        assert_eq!(accent_fills(window, 40., 24., cx), 1, "one accent button");
    });
    ui.finish(cx);
}

/// UI 2.4: a single click selects the card and opens its panel; a double
/// click connects (a new session tab, placeholder in this stage).
#[gpui_kit::test]
fn single_click_opens_the_panel_double_click_a_session(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &sample());
    let [web, ip, ..] = ids[..] else {
        unreachable!()
    };

    ui.frame(cx, |window, cx| {
        assert!(!visible(window, ids::HOST_PANEL));
        window.click(host_card_id(web), cx);
    });
    ui.settle(cx);
    assert_eq!(ui.selected(cx), vec![web]);
    ui.frame(cx, |window, _| {
        assert!(visible(window, ids::HOST_PANEL));
        assert_eq!(
            window.find(ids::HOST_ADDRESS).value(),
            Some("web1.example.com")
        );
        assert_eq!(session_tabs(window), 0, "a single click does not connect");
    });

    // Another card: the panel follows the selection.
    ui.frame(cx, |window, cx| window.click(host_card_id(ip), cx));
    ui.settle(cx);
    assert_eq!(ui.selected(cx), vec![ip]);
    ui.frame(cx, |window, _| {
        assert_eq!(window.find(ids::HOST_ADDRESS).value(), Some("192.0.2.10"));
    });

    ui.frame(cx, |window, cx| window.double_click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| assert_eq!(session_tabs(window), 1));
    assert_eq!(ui.selected(cx), vec![web]);
    ui.finish(cx);
}

/// UI 2.3: Ctrl+click adds or removes a card, Shift+click selects the range
/// in screen order.
#[gpui_kit::test]
fn ctrl_and_shift_click_select_several_cards(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(
        cx,
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
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(c), ctrl(), cx)
    });
    ui.settle(cx);
    assert_eq!(ui.selected(cx), vec![a, c]);
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(a), ctrl(), cx)
    });
    ui.settle(cx);
    assert_eq!(ui.selected(cx), vec![c]);

    ui.frame(cx, |window, cx| window.click(host_card_id(b), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        click_with(window, host_card_id(d), shift(), cx)
    });
    ui.settle(cx);
    let mut selected = ui.selected(cx);
    selected.sort();
    let mut expected = vec![b, c, d];
    expected.sort();
    assert_eq!(selected, expected);
    ui.finish(cx);
}

/// UI 2.4 with lead decision 8: E on the selected card opens the panel with
/// the focus in Address; Enter quick-connects (a session tab).
#[gpui_kit::test]
fn e_edits_and_enter_connects_the_selected_card(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &sample());
    let web = ids[0];
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| window.press("e", cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        assert!(visible(window, ids::HOST_PANEL));
        assert_eq!(window.find(ids::HOST_ADDRESS).focused(), Some(true));
        // The key did not reach the field as text.
        assert_eq!(
            window.find(ids::HOST_ADDRESS).value(),
            Some("web1.example.com")
        );
    });

    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| window.press("enter", cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| assert_eq!(session_tabs(window), 1));
    ui.finish(cx);
}

/// The names of the top-level items of the open context menu, top to
/// bottom: our element id for our items, the label for gpui-kit submenu
/// headers (they carry no id of ours).
fn menu_items(window: &Window, submenu: bool) -> Vec<String> {
    let snapshots = gpui_kit::base::test_support::snapshots(window);
    let in_submenu =
        |path: &[gpui_kit::ElementId]| path.iter().any(|id| id.to_string() == "submenu");
    let mut items: Vec<_> = snapshots
        .iter()
        .filter(|snapshot| snapshot.visible())
        .filter(|snapshot| snapshot.role() == Some(gpui_kit::Role::MenuItem))
        .filter(|snapshot| {
            snapshot
                .path()
                .iter()
                .any(|id| id.to_string() == ids::POPUP_MENU)
        })
        .filter(|snapshot| in_submenu(snapshot.path()) == submenu)
        .collect();
    items.sort_by(|a, b| {
        a.bounds()
            .origin
            .y
            .partial_cmp(&b.bounds().origin.y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items
        .into_iter()
        .filter_map(|item| {
            let ours = snapshots.iter().find_map(|inner| {
                let id = inner.path().last()?.to_string();
                (inner.path().len() > item.path().len()
                    && inner.path().starts_with(item.path())
                    && id.starts_with("menu-"))
                .then_some(id)
            });
            ours.or_else(|| item.label().map(str::to_owned))
        })
        .collect()
}

fn hover_menu_item(window: &mut Window, label: &str, cx: &mut App) {
    window.render_frame(cx);
    let target = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.role() == Some(gpui_kit::Role::MenuItem))
        .find(|snapshot| snapshot.label() == Some(label))
        .unwrap_or_else(|| panic!("menu item {label:?}"));
    let position = target.bounds().center();
    window.dispatch_event(
        gpui_kit::InputEvent::to_platform_input(gpui_kit::MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }),
        cx,
    );
    window.render_frame(cx);
}

/// UI 2.4: right-click opens the card menu, exactly in this order: Quick
/// Connect, Connect ▸ (via SSH, via SFTP, separator, the Alt hint), Add to
/// Workspace ▸, Edit Host Details, Collaborate, Move to ▸, Copy to ▸,
/// Duplicate, Copy Link, Remove. Every item is shown (lead decision 1).
#[gpui_kit::test]
fn context_menu_items_and_order(cx: &mut TestAppContext) {
    use shellpp_desktop::hosts::menu;

    // The model, for one card and for several (every item stays visible).
    for count in [1, 3] {
        assert_eq!(
            menu::entry_ids(&menu::host_menu(count, &[], &[], false)),
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
            ],
            "{count} selected"
        );
    }

    let (ui, ids) = Ui::open(cx, &sample());
    let web = ids[0];
    ui.frame(cx, |window, cx| window.right_click(host_card_id(web), cx));
    ui.settle(cx);
    assert_eq!(ui.selected(cx), vec![web], "right-click selects the card");
    ui.frame(cx, |window, cx| {
        assert!(visible(window, ids::POPUP_MENU));
        assert_eq!(
            menu_items(window, false),
            [
                "menu-quick-connect",
                "Connect",
                "Add to Workspace",
                "menu-edit-host",
                "menu-collaborate",
                "Move to",
                "Copy to",
                "menu-duplicate",
                "menu-copy-link",
                "menu-remove",
            ]
        );
        hover_menu_item(window, "Connect", cx);
    });
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        assert_eq!(
            menu_items(window, true),
            [
                "menu-connect-ssh",
                "menu-connect-sftp",
                "menu-connect-telnet",
                "menu-connect-alt-hint"
            ]
        );
    });
    ui.finish(cx);
}

/// Owner's audit item 13: a toast appears at the bottom of the main area and
/// never covers the right panel or its Connect button.
#[gpui_kit::test]
fn toasts_stay_in_the_main_area(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &sample());
    let web = ids[0];
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    // Copy Link shows "Link copied".
    ui.frame(cx, |window, cx| window.right_click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| window.click("menu-copy-link", cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        let toasts = toasts(window);
        assert!(!toasts.is_empty(), "a toast is shown");
        let panel = bounds(window, ids::RIGHT_PANEL);
        let main = bounds(window, ids::MAIN_AREA);
        for toast in toasts {
            assert!(!toast.intersects(&panel), "{toast:?} over {panel:?}");
            assert!(
                toast.origin.x >= main.origin.x
                    && toast.origin.x + toast.size.width <= main.origin.x + main.size.width,
                "{toast:?} outside {main:?}"
            );
            // 16 px above the bottom of the main area (lead decision 7).
            let gap = (main.origin.y + main.size.height) - (toast.origin.y + toast.size.height);
            assert!(
                (gap.as_f32() - 16.).abs() <= 1.,
                "toast {gap:?} from the bottom"
            );
        }
    });
    // The toast goes away after 5 s.
    ui.wait(cx, std::time::Duration::from_secs(6));
    ui.frame(cx, |window, _| assert!(toasts(window).is_empty()));
    ui.finish(cx);
}

/// The host cards on screen: (left, top, width, height), top to bottom,
/// then left to right.
fn cards(window: &Window) -> Vec<(f32, f32, f32, f32)> {
    numbered_cards(window)
        .into_iter()
        .map(|bounds| {
            (
                bounds.origin.x.as_f32(),
                bounds.origin.y.as_f32(),
                bounds.size.width.as_f32(),
                bounds.size.height.as_f32(),
            )
        })
        .collect()
}

fn numbered_cards(window: &Window) -> Vec<gpui_kit::Bounds<gpui_kit::Pixels>> {
    common::elements_with_prefix(window, "host-card-")
        .into_iter()
        // A card's own id is `host-card-<uuid>` (36 characters after the prefix).
        .filter(|(id, _)| id.len() == "host-card-".len() + 36)
        .map(|(_, snapshot)| snapshot.bounds())
        .collect()
}

/// Checks the grid of design-system 6 / owner's audit item 4 and returns
/// the number of columns: cards 64 px high, 12 px apart in both directions,
/// 240-360 px wide.
fn check_grid(window: &Window) -> usize {
    let cards = cards(window);
    assert!(cards.len() >= 2, "cards on screen");
    let top = cards[0].1;
    let first_row: Vec<_> = cards.iter().filter(|card| card.1 == top).collect();
    for card in &cards {
        assert_eq!(card.3, 64., "card height");
        assert!((240. ..=360.).contains(&card.2), "card width {}", card.2);
    }
    for pair in first_row.windows(2) {
        let gap = pair[1].0 - (pair[0].0 + pair[0].2);
        assert!((gap - 12.).abs() < 0.5, "column gap {gap}");
    }
    if let Some(next) = cards.iter().find(|card| card.1 > top) {
        let gap = next.1 - (top + 64.);
        assert!((gap - 12.).abs() < 0.5, "row gap {gap}");
    }
    first_row.len()
}

/// Owner's audit items 4 and 8: at 1920 px the grid has 5 columns; the
/// 380 px host panel narrows the main area (4 columns) and never overlaps
/// the cards; at 1280 px there are 3 columns.
#[gpui_kit::test]
fn grid_columns_and_the_details_panel_share_the_width(cx: &mut TestAppContext) {
    let hosts: Vec<_> = (0..16)
        .map(|index| json!({"address": format!("h{index:02}.example.com")}))
        .collect();
    let (ui, ids) = Ui::open_sized(cx, 1920., 1200., &hosts);
    ui.frame(cx, |window, _| assert_eq!(check_grid(window), 5));

    ui.frame(cx, |window, cx| window.click(host_card_id(ids[0]), cx));
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        let panel = bounds(window, ids::RIGHT_PANEL);
        assert_eq!(panel.size.width.as_f32(), 380., "--width-details");
        assert_eq!(check_grid(window), 4);
        for card in cards(window) {
            assert!(
                card.0 + card.2 <= panel.origin.x.as_f32(),
                "a card under the panel"
            );
        }
    });
    ui.finish(cx);

    let (ui, _) = Ui::open_sized(cx, 1280., 800., &hosts);
    ui.frame(cx, |window, _| assert_eq!(check_grid(window), 3));
    ui.finish(cx);
}
