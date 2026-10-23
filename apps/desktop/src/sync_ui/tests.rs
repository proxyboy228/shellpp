//! Headless tests of the duplicates from sync (UI 13.7): the hint on the
//! Hosts screen, the side-by-side comparison and its three choices, and
//! Review's way into it. Temporary profiles and the in-memory secret store
//! only; the pairs are listed as a pull would list them
//! (`Core::sync_duplicates_scan`), without a server.

use std::collections::BTreeMap;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use uuid::Uuid;

use super::duplicates::{self, DuplicatesOf};
use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::{TestProfile, generated_password};

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
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
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn visible(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn focused(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> Option<bool> {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.find(id.clone()).focused()
        })
        .unwrap()
    }

    fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    /// The number on the Hosts screen's hint (0: hidden).
    fn hint_count(&self, cx: &mut TestAppContext) -> usize {
        cx.update(|cx| {
            let shell = self.root.read(cx).shell().cloned().expect("shell");
            let screen = shell.read(cx).hosts_screen().clone();
            let hint = screen.read(cx).duplicates_hint().clone();
            hint.read(cx).count()
        })
    }

    /// A host with a saved password.
    fn host(
        &self,
        label: &str,
        address: &str,
        password: &SecretString,
        cx: &mut TestAppContext,
    ) -> Uuid {
        let data = json!({"label": label, "address": address})
            .as_object()
            .cloned()
            .unwrap();
        let secrets = BTreeMap::from([("ssh.password".to_owned(), Some(password.clone()))]);
        self.state.update(cx, |state, cx| {
            state
                .save_host_with_secrets(None, data, secrets, cx)
                .expect("host saved")
                .record_id
        })
    }

    fn live_hosts(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.state.read_with(cx, |state, _| {
            state.hosts().iter().map(|host| host.record_id).collect()
        })
    }

    fn deleted(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.core(cx)
            .recently_deleted_list()
            .expect("trash")
            .into_iter()
            .map(|item| item.record_id)
            .collect()
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// Two hosts of one address created one after the other (the older
/// first), a third one apart, and the pair listed as a pull would.
fn with_a_pair(ui: &Ui, cx: &mut TestAppContext) -> (Uuid, Uuid, [SecretString; 2]) {
    let secrets = [generated_password(), generated_password()];
    let older = ui.host("Web", "dup.example.com", &secrets[0], cx);
    let newer = ui.host("Web (laptop)", "dup.example.com", &secrets[1], cx);
    let _ = ui.host("Other", "other.example.com", &secrets[0], cx);
    ui.settle(cx);
    assert_eq!(ui.hint_count(cx), 0, "no pair before a pull lists one");
    assert!(!ui.visible("hosts-duplicates", cx));
    assert_eq!(ui.core(cx).sync_duplicates_scan().expect("scan"), 1);
    ui.settle(cx);
    (older, newer, secrets)
}

/// The hint shows the records in pairs; Review opens the pair side by
/// side: the older on the left with the focus on Keep left, the fields
/// that differ highlighted on both sides, the others not. Keep left
/// merges into it: the newer goes to Recently deleted, the hint goes away.
#[gpui::test]
fn the_hint_opens_the_pair_and_keep_left_merges_into_the_older(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (older, newer, _) = with_a_pair(&ui, cx);
    assert_eq!(ui.hint_count(cx), 2, "2 hosts may be duplicates");
    assert!(ui.visible("hosts-duplicates", cx));

    ui.click("hosts-duplicates-review", cx);
    assert!(ui.visible("sync-duplicates", cx), "the comparison opened");
    assert!(ui.visible(("dup-left", 0usize), cx) && ui.visible(("dup-right", 0usize), cx));
    for side in ["left", "right"] {
        for path in ["label", "ssh.password"] {
            let id = format!("dup-0-{side}-{path}-differs");
            assert!(ui.visible(gpui::SharedString::from(id.clone()), cx), "{id}");
        }
        let same = format!("dup-0-{side}-address");
        assert!(
            ui.visible(gpui::SharedString::from(same.clone()), cx),
            "{same}"
        );
        assert!(!ui.visible(gpui::SharedString::from(format!("{same}-differs")), cx));
    }
    assert_eq!(
        ui.focused(("dup-keep-left", 0usize), cx),
        Some(true),
        "the older one has the focus"
    );

    ui.click(("dup-keep-left", 0usize), cx);
    assert!(
        !ui.visible("sync-duplicates", cx),
        "closed after the last pair"
    );
    let live = ui.live_hosts(cx);
    assert!(live.contains(&older) && !live.contains(&newer));
    assert_eq!(ui.deleted(cx), vec![newer]);
    assert_eq!(ui.hint_count(cx), 0);
    assert!(!ui.visible("hosts-duplicates", cx));
    ui.finish(cx);
}

/// Keep right merges into the newer one: the older goes to Recently
/// deleted.
#[gpui::test]
fn keep_right_merges_into_the_newer(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (older, newer, _) = with_a_pair(&ui, cx);
    ui.click("hosts-duplicates-review", cx);
    ui.click(("dup-keep-right", 0usize), cx);
    let live = ui.live_hosts(cx);
    assert!(live.contains(&newer) && !live.contains(&older));
    assert_eq!(ui.deleted(cx), vec![older]);
    assert!(!ui.visible("hosts-duplicates", cx));
    ui.finish(cx);
}

/// Keep both keeps both; the hint goes away and the pair is not offered
/// again.
#[gpui::test]
fn keep_both_keeps_both_and_the_hint_goes_away(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (older, newer, _) = with_a_pair(&ui, cx);
    ui.click("hosts-duplicates-review", cx);
    ui.click(("dup-keep-both", 0usize), cx);
    assert!(!ui.visible("sync-duplicates", cx));
    let live = ui.live_hosts(cx);
    assert!(live.contains(&older) && live.contains(&newer));
    assert!(ui.deleted(cx).is_empty());
    assert_eq!(ui.hint_count(cx), 0);
    assert!(!ui.visible("hosts-duplicates", cx));
    assert_eq!(ui.core(cx).sync_duplicates_scan().expect("scan"), 0);
    ui.settle(cx);
    assert!(!ui.visible("hosts-duplicates", cx), "not offered again");
    ui.finish(cx);
}

/// The comparison gets the records with secrets masked by the core: no
/// saved password reaches the dialog.
#[gpui::test]
fn the_comparison_never_holds_a_secret(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (older, newer, secrets) = with_a_pair(&ui, cx);
    let state = ui.state.clone();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            duplicates::open_duplicates(state, DuplicatesOf::Types(duplicates::HOSTS), window, cx)
        })
        .unwrap();
    ui.settle(cx);
    let (items, focused) = cx
        .update_window(ui.handle, |_, window, cx| {
            let dialog = dialog.read(cx);
            (
                dialog.items.clone(),
                dialog.focus_handle().is_focused(window),
            )
        })
        .unwrap();
    assert!(focused);
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].left.record_id, items[0].right.record_id),
        (older, newer)
    );
    let text = serde_json::to_string(&items).expect("json");
    for secret in &secrets {
        assert!(!text.contains(secret.expose_secret()));
    }
    let password = items[0]
        .fields
        .iter()
        .find(|field| field.path == "ssh.password")
        .expect("the password field");
    assert_eq!(
        (password.left.as_str(), password.right.as_str()),
        ("••••••", "••••••")
    );
    ui.finish(cx);
}

/// Review lists the pair with Compare…, which opens the comparison over
/// it; after a choice Review lists nothing.
#[gpui::test]
fn review_opens_the_comparison_of_a_pair(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let _ = with_a_pair(&ui, cx);
    let state = ui.state.clone();
    let review = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_review(state, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    assert!(ui.visible(("sync-duplicate", 0usize), cx));
    ui.click(("sync-compare", 0usize), cx);
    assert!(ui.visible("sync-duplicates", cx));
    ui.click(("dup-keep-both", 0usize), cx);
    assert!(!ui.visible("sync-duplicates", cx));
    assert!(review.read_with(cx, |review, _| review.duplicates.is_empty()));
    assert!(!ui.visible(("sync-duplicate", 0usize), cx));
    ui.finish(cx);
}

/// Snippets has a hint of its own: two snippets of one name and script.
#[gpui::test]
fn snippets_show_their_own_hint(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    let vault_id = ui.state.read_with(cx, |state, _| state.vault_id().unwrap());
    for _ in 0..2 {
        core.snippet_save(shellpp_core::service::snippets::RecordInput {
            vault_id,
            record_id: None,
            data: json!({"label": "Disk", "script": "df -h"})
                .as_object()
                .cloned()
                .unwrap(),
        })
        .expect("snippet saved");
    }
    assert_eq!(core.sync_duplicates_scan().expect("scan"), 1);
    ui.settle(cx);
    assert_eq!(ui.hint_count(cx), 0, "not a pair of hosts");
    cx.update(|cx| {
        let shell = ui.root.read(cx).shell().cloned().expect("shell");
        shell.update(cx, |shell, cx| {
            shell.set_section(crate::shell::Section::Snippets, cx)
        });
    });
    ui.settle(cx);
    assert!(ui.visible("snippet-duplicates", cx));
    ui.click("snippet-duplicates-review", cx);
    assert!(ui.visible(("dup-pair", 0usize), cx));
    ui.click(("dup-keep-both", 0usize), cx);
    assert!(!ui.visible("snippet-duplicates", cx));
    ui.finish(cx);
}
