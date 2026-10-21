//! Owner's stage 1, steps 3.3 and 5.1: Remove asks first, then offers Undo
//! for a few seconds (UI 2.4). Undo brings the removed records back under
//! their ids, with every field and secret, in their place (manual order,
//! parent group), and moves the children of a removed group back into it.
//! Both outcomes are on disk: an undone removal is there after a restart, a
//! removal that was not undone stays removed.
//!
//! Headless GPUI over a temporary profile that the test keeps across the
//! restarts; in-memory secret store.

mod common;

use std::time::Duration;

use common::{Ui, map, numbered, visible};
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use shellpp_desktop::hosts::{group_card_id, host_card_id};
use shellpp_desktop::state::records_edit::{HostDraft, HostSecrets, SecretChange};
use shellpp_desktop::testing::TestProfile;
use uuid::Uuid;

mod ids {
    pub const REMOVE: &str = "menu-remove";
    pub const REMOVE_DIALOG: &str = "remove-dialog";
    pub const REMOVE_CONFIRM: &str = "remove-confirm";
    pub const TOAST_UNDO: &str = "toast-undo";
    pub const TOAST_PREFIX: &str = "toast-";
}

/// The toast lifetime plus a margin (`metrics::TOAST_DURATION` = 5 s).
const AFTER_TOAST: Duration = Duration::from_secs(6);

/// The top-level hosts in manual order.
fn top_level(ui: &Ui, cx: &mut TestAppContext) -> Vec<Uuid> {
    cx.update(|cx| {
        ui.state
            .read(cx)
            .hosts_in(None)
            .iter()
            .map(|host| host.record_id)
            .collect()
    })
}

/// Right-click on the card `card`, Remove, confirm in the dialog.
fn remove_through_the_menu(ui: &Ui, cx: &mut TestAppContext, card: gpui_kit::SharedString) {
    ui.frame(cx, |window, cx| window.right_click(card, cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| window.click(ids::REMOVE, cx));
    ui.settle(cx);
    ui.frame(cx, |window, cx| {
        assert!(visible(window, ids::REMOVE_DIALOG), "Remove asks first");
        window.click(ids::REMOVE_CONFIRM, cx);
    });
    ui.settle(cx);
}

/// A host with fields and a secret in the middle of the manual order: Remove
/// (menu, dialog), Undo in the toast brings it back the same way it was;
/// the restored host is on disk after a restart.
#[gpui_kit::test]
fn undo_brings_a_removed_host_back(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let secret = format!("pw-{}", Uuid::now_v7().simple());
    let ui = Ui::launch(cx, &mut profile);
    let [first, _, last] = ["first", "middle", "last"].map(|name| {
        ui.state.update(cx, |state, cx| {
            state
                .save_host(
                    None,
                    map(json!({"address": format!("{name}.example.com"), "label": name})),
                    cx,
                )
                .expect("host")
                .record_id
        })
    });
    // Replace the middle host by a richer one at the same place.
    let middle = ui.state.update(cx, |state, cx| {
        let draft = HostDraft {
            address: "removed.example.com".into(),
            label: "To be removed".into(),
            tags: vec!["prod".into()],
            notes: "keep these notes".into(),
            port: "2222".into(),
            username: "deploy".into(),
            ..Default::default()
        };
        let secrets = HostSecrets {
            password: SecretChange::Set(SecretString::from(secret.clone())),
            ..Default::default()
        };
        let id = state
            .save_host_draft(None, &draft, &secrets, cx)
            .saved
            .expect("host")
            .record_id;
        state
            .move_records_to(
                vec![id],
                None,
                shellpp_core::service::dto::MovePosition::After { record_id: first },
                cx,
            )
            .expect("place it after the first host");
        id
    });
    let order_before = top_level(&ui, cx);
    let before = ui.host(cx, middle).expect("the host");

    remove_through_the_menu(&ui, cx, host_card_id(middle));
    assert!(ui.host(cx, middle).is_none(), "removed");
    ui.frame(cx, |window, cx| {
        assert!(!visible(window, host_card_id(middle)));
        assert!(visible(window, ids::TOAST_UNDO), "the toast offers Undo");
        window.click(ids::TOAST_UNDO, cx);
    });
    ui.settle(cx);

    let after = ui.host(cx, middle).expect("Undo brought it back");
    assert_eq!(after.data, before.data, "every field");
    assert_eq!(after.group_id, before.group_id);
    assert_eq!(top_level(&ui, cx), order_before, "in its place");
    let revealed = cx
        .update(|cx| ui.state.read(cx).reveal_secret(middle, "ssh.password"))
        .expect("the secret is back");
    assert_eq!(revealed.expose_secret(), secret);
    ui.frame(cx, |window, _| {
        assert!(visible(window, host_card_id(middle)));
        assert!(!visible(window, ids::TOAST_UNDO), "the toast is gone");
    });
    ui.close(cx);

    // Restart: the restored host is stored.
    let ui = Ui::launch(cx, &mut profile);
    let stored = ui.host(cx, middle).expect("after a restart");
    assert_eq!(stored.data, before.data);
    assert_eq!(top_level(&ui, cx), order_before);
    assert!(ui.host(cx, last).is_some());
    ui.close(cx);
}

/// Without Undo the toast goes away after its 5 seconds, Undo is no longer
/// possible, and the host stays removed after a restart.
#[gpui_kit::test]
fn a_removal_without_undo_is_final(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let ui = Ui::launch(cx, &mut profile);
    let [keep, gone] = ["keep", "gone"].map(|name| {
        ui.state.update(cx, |state, cx| {
            state
                .save_host(
                    None,
                    map(json!({"address": format!("{name}.example.com")})),
                    cx,
                )
                .expect("host")
                .record_id
        })
    });
    remove_through_the_menu(&ui, cx, host_card_id(gone));
    assert!(ui.host(cx, gone).is_none());
    ui.wait(cx, AFTER_TOAST);
    ui.frame(cx, |window, _| {
        assert!(!visible(window, ids::TOAST_UNDO));
        assert!(numbered(window, ids::TOAST_PREFIX).is_empty());
    });
    // The undo window has closed.
    let late = ui
        .state
        .update(cx, |state, cx| state.restore_records(vec![gone], cx));
    assert!(late.is_err(), "Undo after the window fails");
    ui.close(cx);

    let ui = Ui::launch(cx, &mut profile);
    assert!(ui.host(cx, gone).is_none(), "still removed after a restart");
    assert!(ui.host(cx, keep).is_some());
    ui.close(cx);
}

/// Removing a group moves its hosts one level up; Undo brings the group
/// back and moves them into it again, also after a restart.
#[gpui_kit::test]
fn undo_of_a_group_removal_moves_its_hosts_back(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let ui = Ui::launch(cx, &mut profile);
    let (group, inside) = ui.state.update(cx, |state, cx| {
        let group = state
            .save_group(None, map(json!({"name": "Databases", "color": "jade"})), cx)
            .expect("group")
            .record_id;
        let inside = state
            .save_host(
                None,
                map(json!({"address": "db1.example.com", "group_id": group})),
                cx,
            )
            .expect("host")
            .record_id;
        (group, inside)
    });
    let group_before = cx
        .update(|cx| ui.state.read(cx).group(group).cloned())
        .expect("group");

    remove_through_the_menu(&ui, cx, group_card_id(group));
    cx.update(|cx| {
        let state = ui.state.read(cx);
        assert!(state.group(group).is_none(), "the group is removed");
        assert_eq!(
            state.host(inside).map(|host| host.group_id),
            Some(None),
            "its host moved to the top level"
        );
    });
    ui.frame(cx, |window, cx| window.click(ids::TOAST_UNDO, cx));
    ui.settle(cx);
    cx.update(|cx| {
        let state = ui.state.read(cx);
        let restored = state.group(group).expect("the group is back");
        assert_eq!(restored.data, group_before.data);
        assert_eq!(
            state.host(inside).and_then(|host| host.group_id),
            Some(group),
            "its host is inside again"
        );
    });
    ui.close(cx);

    let ui = Ui::launch(cx, &mut profile);
    assert_eq!(
        ui.host(cx, inside).and_then(|host| host.group_id),
        Some(group)
    );
    ui.close(cx);
}
