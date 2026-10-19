//! Headless UI tests of the host panel's saving rule (owner's step 5; UI
//! 1.3, 3.2): an edit is saved to the store after the autosave delay, an
//! invalid value is shown and not saved, a new host is a draft until its
//! address is valid and closing an empty draft discards it. GPUI
//! `TestAppContext`, no real window, a temporary profile per test.

mod common;

use common::{AFTER_DEBOUNCE, BEFORE_DEBOUNCE, Ui, visible};
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;
use shellpp_desktop::hosts::host_card_id;
use shellpp_desktop::shell::Panel;

mod ids {
    pub const NEW_HOST: &str = "new-host";
    pub const HOST_PANEL: &str = "host-panel";
    pub const HOST_ADDRESS: &str = "host-address";
    pub const HOST_LABEL: &str = "host-label";
    pub const PANEL_CLOSE: &str = "panel-close";
}

/// Replaces the text of the input `id`.
fn replace_text(ui: &Ui, cx: &mut TestAppContext, id: &'static str, text: &str) {
    ui.frame(cx, |window, cx| {
        window.click(id, cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    });
    ui.settle(cx);
}

/// UI 1.3: no Save button; an edit is written to the store once the
/// 500 ms autosave delay has passed, not before.
#[gpui_kit::test]
fn edits_are_saved_after_the_autosave_delay(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(
        cx,
        &[
            json!({"address": "192.0.2.10"}),
            json!({"address": "web1.example.com", "label": "Web", "tags": ["prod"], "notes": "keep me"}),
        ],
    );
    let web = ids[1];
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        assert!(visible(window, ids::HOST_PANEL));
        assert_eq!(window.find(ids::HOST_LABEL).value(), Some("Web"));
        window.click(ids::HOST_LABEL, cx);
        window.input(" 1", cx);
    });

    ui.wait(cx, BEFORE_DEBOUNCE);
    assert_eq!(ui.host(cx, web).unwrap().label.as_deref(), Some("Web"));
    ui.wait(cx, AFTER_DEBOUNCE);
    let host = ui.host(cx, web).unwrap();
    assert_eq!(host.label.as_deref(), Some("Web 1"));
    // Only the edited member changed.
    assert_eq!(host.address, "web1.example.com");
    assert_eq!(host.tags, ["prod"]);
    assert_eq!(host.data["notes"], "keep me");

    let shell = ui.shell(cx);
    cx.update(|cx| {
        assert!(matches!(shell.read(cx).panel(), Some(Panel::Host(_))));
    });
    ui.finish(cx);
}

/// UI 1.3: an invalid value is highlighted with its reason under the field
/// and is not saved; the last valid value stays in the store. A later valid
/// value is saved.
#[gpui_kit::test]
fn invalid_values_are_not_saved(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &[json!({"address": "web1.example.com"})]);
    let web = ids[0];
    ui.frame(cx, |window, cx| window.click(host_card_id(web), cx));
    ui.settle(cx);

    replace_text(&ui, cx, ids::HOST_ADDRESS, "bad host");
    ui.wait(cx, AFTER_DEBOUNCE);
    assert_eq!(ui.host(cx, web).unwrap().address, "web1.example.com");
    assert!(address_error(&ui, cx), "the reason is shown under Address");

    replace_text(&ui, cx, ids::HOST_ADDRESS, "web2.example.com");
    ui.wait(cx, AFTER_DEBOUNCE);
    assert_eq!(ui.host(cx, web).unwrap().address, "web2.example.com");
    assert!(!address_error(&ui, cx));
    ui.finish(cx);
}

/// Whether the open host panel marks Address as invalid (the panel shows
/// the reason under the field for every entry of its error list).
fn address_error(ui: &Ui, cx: &mut TestAppContext) -> bool {
    let shell = ui.shell(cx);
    cx.update(|cx| match shell.read(cx).panel() {
        Some(Panel::Host(panel)) => panel.read(cx).errors().contains_key("address"),
        _ => panic!("the host panel is open"),
    })
}

/// UI 1.3, 3.2: New host opens an empty panel with the focus in Address;
/// without an address nothing is written, and closing the panel discards
/// the draft silently.
#[gpui_kit::test]
fn an_empty_draft_is_discarded(cx: &mut TestAppContext) {
    let (ui, _) = Ui::open(cx, &[json!({"address": "web1.example.com"})]);
    let before = ui.hosts(cx).len();
    ui.frame(cx, |window, cx| window.click(ids::NEW_HOST, cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        assert!(visible(window, ids::HOST_PANEL));
        assert_eq!(window.find(ids::HOST_ADDRESS).focused(), Some(true));
        assert_eq!(window.find(ids::HOST_ADDRESS).value(), Some(""));
        window.click(ids::HOST_LABEL, cx);
        window.input("Draft only", cx);
    });
    ui.wait(cx, AFTER_DEBOUNCE);
    assert_eq!(ui.hosts(cx).len(), before, "a draft is not saved");

    ui.frame(cx, |window, cx| window.click(ids::PANEL_CLOSE, cx));
    ui.settle(cx);
    ui.wait(cx, AFTER_DEBOUNCE);
    ui.frame(cx, |window, _| assert!(!visible(window, ids::HOST_PANEL)));
    assert_eq!(ui.hosts(cx).len(), before, "closing discards the draft");
    ui.finish(cx);
}

/// UI 3.2: the host is created as soon as a valid address is entered, it
/// appears in the list selected, and later edits update the same record.
#[gpui_kit::test]
fn a_new_host_is_created_by_its_first_valid_address(cx: &mut TestAppContext) {
    let (ui, _) = Ui::open(cx, &[json!({"address": "web1.example.com"})]);
    let before = ui.hosts(cx).len();
    ui.frame(cx, |window, cx| window.click(ids::NEW_HOST, cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| window.input("new.example.com", cx));
    ui.wait(cx, AFTER_DEBOUNCE);

    let hosts = ui.hosts(cx);
    assert_eq!(hosts.len(), before + 1);
    let created = hosts
        .iter()
        .find(|host| host.address == "new.example.com")
        .expect("the new host")
        .record_id;
    assert_eq!(ui.selected(cx), vec![created]);
    ui.frame(cx, |window, _| {
        assert!(visible(window, host_card_id(created)))
    });

    replace_text(&ui, cx, ids::HOST_LABEL, "New one");
    ui.wait(cx, AFTER_DEBOUNCE);
    assert_eq!(ui.hosts(cx).len(), before + 1, "no second record");
    assert_eq!(
        ui.host(cx, created).unwrap().label.as_deref(),
        Some("New one")
    );
    ui.finish(cx);
}
