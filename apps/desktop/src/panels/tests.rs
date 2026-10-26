//! Headless tests of the host and group panels (GPUI `TestAppContext`, a
//! temporary profile, no real window): block order, the saving rule
//! (draft, autosave, invalid values), inheritance placeholders, the menus and
//! the Connect button.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::ValidationReason;
use shellpp_core::service::dto::TagColor;
use uuid::Uuid;

use super::menus::{
    GROUP_MENU, GroupMenuAction, HOST_PANEL_MENU, HostPanelAction, group_move_choices,
    remove_group_body,
};
use super::{GroupPanel, HostPanel, PanelEvent, PanelRequest, strings};
use crate::state::AppState;
use crate::state::records_edit::{BackspaceMode, ProxyKind};
use crate::testing::TestProfile;
use crate::theme::metrics;

/// A bit more than the autosave debounce.
const AFTER_DEBOUNCE: Duration = Duration::from_millis(600);

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn setup(cx: &mut TestAppContext) -> Entity<AppState> {
    cx.update(|cx| {
        crate::testing::init_ui(cx);
    });
    cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx))
}

/// A window as wide as the details panel and tall enough to show every block.
fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(metrics::at_100(metrics::WIDTH_DETAILS), px(3000.)),
        ))),
        ..Default::default()
    }
}

fn open_host_panel(
    cx: &mut TestAppContext,
    state: &Entity<AppState>,
    record_id: Option<Uuid>,
) -> (AnyWindowHandle, Entity<HostPanel>) {
    let state = state.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| HostPanel::new(state, record_id, window, cx))
        })
    })
    .unwrap()
}

fn open_group_panel(
    cx: &mut TestAppContext,
    state: &Entity<AppState>,
    record_id: Option<Uuid>,
) -> (AnyWindowHandle, Entity<GroupPanel>) {
    let state = state.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| GroupPanel::new(state, record_id, window, cx))
        })
    })
    .unwrap()
}

/// Events a panel emitted, in order.
type Seen<T> = Rc<RefCell<Vec<T>>>;

/// Collects the events of a panel entity.
fn record_events<V>(
    cx: &mut TestAppContext,
    panel: &Entity<V>,
) -> (Seen<PanelEvent>, Seen<PanelRequest>)
where
    V: gpui::EventEmitter<PanelEvent> + gpui::EventEmitter<PanelRequest> + 'static,
{
    let events = Rc::new(RefCell::new(Vec::new()));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let (seen_events, seen_requests) = (events.clone(), requests.clone());
    cx.update(|cx| {
        cx.subscribe(panel, move |_, event: &PanelEvent, _| {
            seen_events.borrow_mut().push(event.clone())
        })
        .detach();
        cx.subscribe(panel, move |_, request: &PanelRequest, _| {
            seen_requests.borrow_mut().push(request.clone())
        })
        .detach();
    });
    (events, requests)
}

fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(AFTER_DEBOUNCE);
    cx.run_until_parked();
}

/// Confirms the open dialog (Enter = OK) once its opening animation is over.
fn confirm_dialog(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    use gpui_kit::component::WindowExt as _;
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.has_active_dialog(cx), "the removal asks first");
        assert!(window.find("ok").visible());
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!window.has_active_dialog(cx));
    })
    .unwrap();
}

fn top(window: &gpui::Window, id: &'static str) -> gpui::Pixels {
    window.find(id).bounds().origin.y
}

fn assert_top_to_bottom(window: &gpui::Window, ids: &[&'static str]) {
    for pair in ids.windows(2) {
        assert!(
            top(window, pair[0]) < top(window, pair[1]),
            "{} must be above {}",
            pair[0],
            pair[1]
        );
    }
}

#[gpui_kit::test]
fn host_panel_blocks_follow_ui_3_1(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                map(json!({"address": "192.0.2.1", "label": "Web"})),
                cx,
            )
            .unwrap()
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).title(), strings::HOST_DETAILS);
        assert_eq!(window.find("host-label").value(), Some("Web"));
        // UI 3.1: Vault, Address, General, Share, SSH, Add Telnet, Connect.
        assert_top_to_bottom(
            window,
            &[
                "host-block-vault",
                "host-block-address",
                "host-block-general",
                "host-block-share",
                "host-block-ssh",
                "host-block-telnet",
                "host-connect",
            ],
        );
        // General (lead decision 4): Label, Parent Group, Tags, Color,
        // Backspace, Notes.
        assert_top_to_bottom(
            window,
            &[
                "host-label",
                "host-parent-group",
                "host-tags",
                "host-color",
                "host-backspace-default",
                "host-notes",
            ],
        );
        // SSH: the port sits on the protocol line, above the username.
        assert_top_to_bottom(window, &["host-port", "host-username", "host-password"]);
        // Design-system 6: fields are 36 px (the kit's single-line default
        // is 28 px with our 14 px rem).
        for id in ["host-address", "host-label", "host-port", "host-username"] {
            assert_eq!(
                window.find(id).bounds().size.height,
                metrics::at_100(metrics::HEIGHT_CONTROL),
                "{id}"
            );
        }
        assert!(window.try_find("host-more").is_none());
        window.click("host-show-more", cx);
        window.render_frame(cx);
        // Every "Show more" field of UI 3.1; Mosh is not shown until it is
        // built (D204).
        assert!(window.try_find("host-mosh").is_none());
        assert_top_to_bottom(
            window,
            &[
                "host-agent-forwarding",
                "host-startup-snippet",
                "host-jump-hosts",
                "host-proxy-none",
                "host-env",
                "host-charset",
                "host-keepalive",
                "host-history-default",
                "host-output-default",
                "host-terminal-theme",
            ],
        );
        // There is no save button.
        assert!(window.try_find("host-save").is_none());
    })
    .unwrap();
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn new_host_is_a_draft_until_the_address_is_valid(cx: &mut TestAppContext) {
    let state = setup(cx);
    let (handle, panel) = open_host_panel(cx, &state, None);
    let (events, _) = record_events(cx, &panel);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).title(), strings::NEW_HOST);
        // New host: the focus is in Address (UI 3.1).
        assert_eq!(window.find("host-address").focused(), Some(true));
        assert!(!panel.read(cx).can_connect());
        window.input("bad host", cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Invalid: highlighted with a reason, nothing written.
        assert_eq!(
            panel.read(cx).errors().get("address"),
            Some(&ValidationReason::Invalid)
        );
        assert!(state.read(cx).hosts().is_empty());
        assert!(panel.read(cx).record_id().is_none());
        for _ in 0.." host".len() {
            window.press("backspace", cx);
        }
    })
    .unwrap();
    settle(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The first valid address creates the host and selects it (UI 3.2).
        let id = panel.read(cx).record_id().expect("created");
        let state = state.read(cx);
        assert_eq!(state.hosts().len(), 1);
        assert_eq!(state.host(id).unwrap().address, "bad");
        assert!(state.selection().contains(&id));
        assert!(panel.read(cx).errors().is_empty());
        assert_eq!(panel.read(cx).title(), strings::HOST_DETAILS);
        assert!(panel.read(cx).can_connect());
    })
    .unwrap();
    assert!(events.borrow().is_empty());
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn closing_an_empty_draft_discards_it(cx: &mut TestAppContext) {
    let state = setup(cx);
    let (handle, panel) = open_host_panel(cx, &state, None);
    let (events, _) = record_events(cx, &panel);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input("Only a label", cx);
        // Collapse before the debounce: the draft is dropped silently.
        window.click("panel-close", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        assert!(state.read(cx).hosts().is_empty());
        assert!(!panel.read(cx).has_pending_save());
    });
    assert_eq!(events.borrow().as_slice(), &[PanelEvent::Close]);
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn edits_autosave_and_invalid_values_are_not_saved(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                map(json!({"address": "192.0.2.2", "label": "Web"})),
                cx,
            )
            .unwrap()
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input(" 2", cx);
        window.click("host-port", cx);
        window.input("70000", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        // Before the debounce nothing is written; the port is already flagged.
        assert!(panel.read(cx).has_pending_save());
        assert_eq!(
            state
                .read(cx)
                .host(host.record_id)
                .unwrap()
                .label
                .as_deref(),
            Some("Web")
        );
        assert_eq!(
            panel.read(cx).errors().get("ssh.port"),
            Some(&ValidationReason::OutOfRange)
        );
    });
    settle(cx);
    cx.update(|cx| {
        let stored = state.read(cx).host(host.record_id).unwrap().clone();
        // The valid label is saved, the invalid port is not.
        assert_eq!(stored.label.as_deref(), Some("Web 2"));
        assert_eq!(stored.port, None);
        assert!(panel.read(cx).errors().contains_key("ssh.port"));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..3 {
            window.press("backspace", cx);
        }
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        assert_eq!(state.read(cx).host(host.record_id).unwrap().port, Some(70));
        assert!(panel.read(cx).errors().is_empty());
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn general_controls_save(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({"address": "192.0.2.3"})), cx)
            .unwrap()
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Graphite is the default color.
        assert_eq!(panel.read(cx).draft().color, TagColor::Graphite);
        window.click("host-color-ocean", cx);
        window.click("host-backspace-ctrl_h", cx);
        window.click("host-tag-input", cx);
        window.input("prod", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-tag-0").visible());
        // Parent Group: search and create a group on the fly.
        window.click("host-parent-group", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("host-parent-group-search").focused(),
            Some(true)
        );
        window.input("Staging", cx);
        window.render_frame(cx);
        window.click("host-parent-group-create", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        let state = state.read(cx);
        let stored = state.host(host.record_id).unwrap();
        assert_eq!(stored.color, TagColor::Ocean);
        assert_eq!(stored.tags, vec!["prod".to_owned()]);
        assert_eq!(stored.data["backspace"], json!("ctrl_h"));
        let group = state.groups().first().cloned().expect("group created");
        assert_eq!(group.name, "Staging");
        assert_eq!(stored.group_id, Some(group.record_id));
        assert_eq!(panel.read(cx).draft().backspace, BackspaceMode::CtrlH);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-tag-remove-0", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        assert!(state.read(cx).host(host.record_id).unwrap().tags.is_empty());
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn show_more_settings_save(cx: &mut TestAppContext) {
    let state = setup(cx);
    let (jump, host) = state.update(cx, |state, cx| {
        let jump = state
            .save_host(None, map(json!({"address": "bastion.example.com"})), cx)
            .unwrap();
        let host = state
            .save_host(None, map(json!({"address": "192.0.2.4"})), cx)
            .unwrap();
        (jump, host)
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        window.click("host-agent-forwarding", cx);
        window.click("host-history-off", cx);
        window.click("host-jump-add", cx);
        window.render_frame(cx);
        window.input("bastion", cx);
        window.render_frame(cx);
        window.click(format!("host-jump-{}", jump.record_id), cx);
        window.render_frame(cx);
        window.click("host-proxy-socks5", cx);
        window.render_frame(cx);
        window.click("host-proxy-address", cx);
        window.input("proxy.example.com", cx);
        window.click("host-proxy-port", cx);
        window.input("1080", cx);
        window.click("host-env-add", cx);
        window.render_frame(cx);
        window.input("LANG", cx);
        window.click("host-env-value-0", cx);
        window.input("C", cx);
        window.click("host-keepalive", cx);
        window.input("30", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        let stored = state.read(cx).host(host.record_id).unwrap().clone();
        let ssh = &stored.data["ssh"];
        assert_eq!(ssh["agent_forwarding"], json!(true));
        assert_eq!(ssh["jump_chain"], json!([jump.record_id]));
        assert_eq!(
            ssh["proxy"],
            json!({"type": "socks5", "address": "proxy.example.com", "port": 1080})
        );
        assert_eq!(ssh["env"], json!([{"name": "LANG", "value": "C"}]));
        assert_eq!(ssh["keepalive_s"], json!(30));
        assert_eq!(stored.data["record_history"], json!("off"));
        assert_eq!(panel.read(cx).draft().proxy_kind, ProxyKind::Socks5);
        assert!(panel.read(cx).errors().is_empty());
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn inherited_values_show_from_group(cx: &mut TestAppContext) {
    let state = setup(cx);
    let (group, host) = state.update(cx, |state, cx| {
        let group = state
            .save_group(
                None,
                map(json!({"name": "Production", "ssh": {"port": 2222, "username": "ops"}})),
                cx,
            )
            .unwrap();
        let host = state
            .save_host(
                None,
                map(json!({"address": "192.0.2.5", "group_id": group.record_id})),
                cx,
            )
            .unwrap();
        (group, host)
    });
    let (_, panel) = open_host_panel(cx, &state, Some(host.record_id));
    cx.update(|cx| {
        let (port, username) = panel.read(cx).ssh_placeholders(cx);
        assert_eq!(port.as_ref(), strings::inherited("2222", "Production"));
        assert_eq!(username.as_ref(), strings::inherited("ops", "Production"));
        assert!(port.contains("from group Production"));
    });
    // A subgroup panel shows the same inherited values.
    let (_, group_panel) = open_group_panel(cx, &state, None);
    group_panel.update(cx, |panel, cx| {
        assert!(panel.record_id().is_none());
        let _ = group.record_id;
        let (port, _) = panel.ssh_placeholders(cx);
        // A new group at the top level inherits nothing.
        assert_eq!(port.as_ref(), strings::DEFAULT_PORT);
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn connect_is_the_accent_button_unless_search_has_an_address(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({"address": "192.0.2.6"})), cx)
            .unwrap()
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    let (_, requests) = record_events(cx, &panel);
    cx.update(|cx| assert!(panel.read(cx).connect_is_accent()));
    panel.update(cx, |panel, cx| panel.set_search_has_address(true, cx));
    cx.update(|cx| assert!(!panel.read(cx).connect_is_accent()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-connect", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        requests.borrow().as_slice(),
        &[PanelRequest::Connect(host.record_id)]
    );
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn menus_keep_the_spec_order(cx: &mut TestAppContext) {
    // UI 3.3 and 3.4.
    assert_eq!(
        HOST_PANEL_MENU,
        [
            HostPanelAction::Connect,
            HostPanelAction::AddTelnet,
            HostPanelAction::Duplicate,
            HostPanelAction::Remove
        ]
    );
    assert_eq!(
        HOST_PANEL_MENU.map(HostPanelAction::label),
        ["Connect", "Add Telnet", "Duplicate", "Remove"]
    );
    assert_eq!(
        GROUP_MENU.map(GroupMenuAction::label),
        ["Open", "Edit", "Move to", "Remove"]
    );
    assert!(HostPanelAction::Remove.is_danger() && GroupMenuAction::Remove.is_danger());
    assert_ne!(remove_group_body(true), remove_group_body(false));

    let state = setup(cx);
    let (_, draft) = open_host_panel(cx, &state, None);
    cx.update(|cx| {
        let draft = draft.read(cx);
        // A draft can be neither duplicated nor removed.
        assert!(!draft.menu_action_enabled(HostPanelAction::Duplicate));
        assert!(!draft.menu_action_enabled(HostPanelAction::Remove));
        assert!(!draft.menu_action_enabled(HostPanelAction::Connect));
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn duplicate_and_remove_from_the_host_menu(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                map(json!({"address": "192.0.2.7", "label": "Db"})),
                cx,
            )
            .unwrap()
    });
    let (handle, panel) = open_host_panel(cx, &state, Some(host.record_id));
    let (events, requests) = record_events(cx, &panel);
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.run_menu_action(HostPanelAction::Duplicate, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let copy = cx.update(|cx| {
        let hosts = state.read(cx).hosts();
        assert_eq!(hosts.len(), 2);
        hosts
            .iter()
            .find(|item| item.record_id != host.record_id)
            .cloned()
            .unwrap()
    });
    assert_eq!(copy.label.as_deref(), Some("Db Copy"));
    cx.update(|cx| assert!(state.read(cx).selection().contains(&copy.record_id)));
    assert!(
        requests
            .borrow()
            .contains(&PanelRequest::OpenHost(copy.record_id))
    );
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.run_menu_action(HostPanelAction::Remove, window, cx)
        });
    })
    .unwrap();
    confirm_dialog(cx, handle);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(state.read(cx).host(host.record_id).is_none());
    });
    assert!(
        requests
            .borrow()
            .contains(&PanelRequest::Removed(vec![host.record_id]))
    );
    assert!(events.borrow().contains(&PanelEvent::Close));
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn group_panel_creates_edits_and_moves(cx: &mut TestAppContext) {
    let state = setup(cx);
    let parent = state.update(cx, |state, cx| {
        let parent = state
            .save_group(None, map(json!({"name": "Production"})), cx)
            .unwrap();
        state.set_location(Some(parent.record_id), cx);
        parent
    });
    let (handle, panel) = open_group_panel(cx, &state, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).title(), strings::NEW_GROUP);
        assert_eq!(window.find("group-name").focused(), Some(true));
        assert_top_to_bottom(
            window,
            &[
                "group-block-vault",
                "group-block-name",
                "group-block-parent",
                "group-block-color",
                "group-block-ssh",
            ],
        );
        window.input("Databases", cx);
        window.click("group-port", cx);
        window.input("0", cx);
        window.click("group-color-plum", cx);
    })
    .unwrap();
    settle(cx);
    let id = cx.update(|cx| {
        let id = panel.read(cx).record_id().expect("created");
        let group = state.read(cx).group(id).unwrap().clone();
        // Created inside the current location; the invalid port is not saved.
        assert_eq!(group.name, "Databases");
        assert_eq!(group.parent_id, Some(parent.record_id));
        assert_eq!(group.color, TagColor::Plum);
        assert!(group.data["ssh"].get("port").is_none());
        assert_eq!(
            panel.read(cx).errors().get("ssh.port"),
            Some(&ValidationReason::OutOfRange)
        );
        id
    });
    cx.update(|cx| {
        // Move to never offers the group itself or its subtree.
        let choices = group_move_choices(state.read(cx), id);
        assert_eq!(choices.first().map(|choice| choice.0), Some(None));
        assert!(choices.iter().all(|(target, _)| *target != Some(id)));
        // The parent is the current place, not a target.
        assert!(
            choices
                .iter()
                .all(|(target, _)| *target != Some(parent.record_id))
        );
        let choices = group_move_choices(state.read(cx), parent.record_id);
        assert!(choices.iter().all(|(target, _)| *target != Some(id)));
    });
    panel.update(cx, |panel, cx| panel.move_to(None, cx));
    cx.update(|cx| {
        assert_eq!(state.read(cx).group(id).unwrap().parent_id, None);
        assert_eq!(panel.read(cx).draft().parent_id, None);
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn removing_a_non_empty_group_moves_its_children_up(cx: &mut TestAppContext) {
    let state = setup(cx);
    let (outer, inner, host) = state.update(cx, |state, cx| {
        let outer = state
            .save_group(None, map(json!({"name": "Outer"})), cx)
            .unwrap();
        let inner = state
            .save_group(
                None,
                map(json!({"name": "Inner", "parent_id": outer.record_id})),
                cx,
            )
            .unwrap();
        let host = state
            .save_host(
                None,
                map(json!({"address": "192.0.2.8", "group_id": inner.record_id})),
                cx,
            )
            .unwrap();
        (outer, inner, host)
    });
    let (handle, panel) = open_group_panel(cx, &state, Some(inner.record_id));
    let (events, requests) = record_events(cx, &panel);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(panel.read(cx).title(), strings::GROUP_DETAILS);
        panel.update(cx, |panel, cx| {
            panel.run_menu_action(GroupMenuAction::Remove, window, cx)
        });
    })
    .unwrap();
    confirm_dialog(cx, handle);
    cx.run_until_parked();
    cx.update(|cx| {
        let state = state.read(cx);
        assert!(state.group(inner.record_id).is_none());
        // The host moved one level up, into Outer.
        assert_eq!(
            state.host(host.record_id).unwrap().group_id,
            Some(outer.record_id)
        );
    });
    assert!(
        requests
            .borrow()
            .contains(&PanelRequest::Removed(vec![inner.record_id]))
    );
    assert!(events.borrow().contains(&PanelEvent::Close));
    state.update(cx, |state, _| state.shutdown());
}

/// UI 3.1 (stage 3): hosts with the same username and key show the hint;
/// "Create an identity" moves them onto one identity, and an autosave of
/// the open host afterwards keeps it.
#[gpui_kit::test]
fn the_identity_hint_moves_shared_credentials_onto_one_identity(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::{KeyGenerateInput, RecordRef, RecordView};
    let state = setup(cx);
    let core = state.read_with(cx, |state, _| state.core().cloned().unwrap());
    let vault_id = state.read_with(cx, |state, _| state.vault_id().unwrap());
    let key = core
        .key_generate(KeyGenerateInput {
            vault_id,
            label: "ops key".into(),
            kind: Default::default(),
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let save = |state: &mut AppState, address: &str, cx: &mut gpui::Context<AppState>| {
        state
            .save_host(
                None,
                map(json!({
                    "address": address,
                    "ssh": {"username": "ops", "key_id": key.record_id.to_string()}
                })),
                cx,
            )
            .unwrap()
            .record_id
    };
    let first = state.update(cx, |state, cx| save(state, "192.0.2.31", cx));
    let second = state.update(cx, |state, cx| save(state, "192.0.2.32", cx));
    cx.run_until_parked();

    let (handle, panel) = open_host_panel(cx, &state, Some(first));
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find("host-identity-hint")
                .is_some_and(|found| found.visible())
        })
        .unwrap()
    };
    assert!(shown(cx), "two hosts share the username and the key");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-identity-hint-create", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let identities = core.identities_list().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].key_id, Some(key.record_id));
    let identity = identities[0].record_id;
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft().identity_id, Some(identity));
        assert_eq!(panel.draft().key_id, None);
        assert!(panel.draft().username.is_empty());
    });
    assert!(
        !shown(cx),
        "no hint once the identity holds the credentials"
    );

    // An edit autosaves the open host without undoing the merge.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input("Ops 1", cx);
    })
    .unwrap();
    settle(cx);
    for id in [first, second] {
        let RecordView::Host(host) = core
            .record_get(RecordRef {
                vault_id,
                record_id: id,
            })
            .unwrap()
        else {
            panic!("a host");
        };
        let ssh = &host.data["ssh"];
        assert_eq!(ssh["identity_id"], json!(identity.to_string()), "{id}");
        assert!(ssh.get("key_id").is_none_or(Value::is_null));
        assert!(ssh.get("username").is_none_or(Value::is_null));
    }
}

/// The placeholders of a host follow the connection (data-model 8.3): a
/// group's identity supplies the username a host inherits.
#[gpui_kit::test]
fn a_group_identity_supplies_the_inherited_username(cx: &mut TestAppContext) {
    use shellpp_core::model::identity::PasswordChange;
    use shellpp_core::service::keychain::IdentitySaveInput;
    let state = setup(cx);
    let core = state.read_with(cx, |state, _| state.core().cloned().unwrap());
    let vault_id = state.read_with(cx, |state, _| state.vault_id().unwrap());
    let identity = core
        .identity_save(IdentitySaveInput {
            vault_id,
            record_id: None,
            label: "ops".into(),
            username: "ops".into(),
            key_id: None,
            password: PasswordChange::Keep,
            base: None,
        })
        .unwrap();
    let group = state.update(cx, |state, cx| {
        state
            .save_group(
                None,
                map(
                    json!({"name": "prod", "ssh": {"identity_id": identity.record_id.to_string()}}),
                ),
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    let inherited = state.read_with(cx, |state, _| state.inherited_ssh(Some(group.record_id)));
    let username = inherited.username.expect("from the group's identity");
    assert_eq!(username.value, "ops");
    assert_eq!(username.group_id, group.record_id);
}
