//! Headless tests of the sync conflict comparison (UI 13.6): a conflict
//! copy is written through the core's test hook (no sync server), as a
//! merge writes it; temporary profiles only.

use std::time::{Duration, SystemTime};

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, SharedString, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

/// A secret no other text in the window can hold.
fn secret() -> String {
    let bytes: [u8; 12] = rand_bytes();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rand_bytes() -> [u8; 12] {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).unwrap();
    bytes
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
            .unwrap();
        let ui = Self {
            state,
            handle,
            _root: root,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: impl Into<gpui::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.settle(cx);
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    /// A host "web" at 192.0.2.10:22 with a saved password.
    fn host(&self, password: &str, cx: &mut TestAppContext) -> Uuid {
        let secrets = std::collections::BTreeMap::from([(
            "ssh.password".to_owned(),
            Some(SecretString::from(password.to_owned())),
        )]);
        let id = self.state.update(cx, |state, cx| {
            state
                .save_host_with_secrets(
                    None,
                    map(json!({"address": "192.0.2.10", "label": "web", "ssh": {"port": 22}})),
                    secrets,
                    cx,
                )
                .unwrap()
                .record_id
        });
        self.settle(cx);
        id
    }

    /// Sync kept `losing` (`None`: a deletion) of `host` from another
    /// device as a conflict copy.
    fn conflict(&self, host: Uuid, losing: Option<Value>, cx: &mut TestAppContext) -> Uuid {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let id = self
            .core(cx)
            .sync_conflict_insert_for_test(host, losing.map(map), Uuid::now_v7(), at)
            .unwrap();
        self.settle(cx);
        id
    }

    fn label_of(&self, host: Uuid, cx: &mut TestAppContext) -> Option<String> {
        self.state.read_with(cx, |state, _| {
            state
                .hosts()
                .iter()
                .find(|item| item.record_id == host)
                .map(|item| item.title().to_owned())
        })
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn badge(host: Uuid) -> String {
    crate::hosts::host_conflict_badge_id(host).to_string()
}

/// The badge of a conflicted host card opens the comparison: both
/// versions side by side with their heads, the differing fields marked on
/// both sides, the equal ones not; passwords masked. "Keep other" replaces
/// the host with the other version and the badge goes away.
#[gpui::test]
fn badge_opens_the_comparison_and_keep_other_replaces(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (current_secret, other_secret) = (secret(), secret());
    let host = ui.host(&current_secret, cx);
    assert!(!ui.visible(&badge(host), cx), "no conflict yet");
    ui.conflict(
        host,
        Some(json!({
            "address": "192.0.2.10",
            "label": "web-2",
            "ssh": {"port": 2222, "password": other_secret},
        })),
        cx,
    );
    assert!(ui.visible(&badge(host), cx), "the card shows the badge");

    ui.click(SharedString::from(badge(host)), cx);
    assert!(ui.visible("sync-compare", cx), "the comparison opened");
    assert!(ui.visible("sync-compare-current", cx));
    assert!(ui.visible("sync-compare-other", cx));
    for side in ["current", "other"] {
        for path in ["label", "ssh.port", "ssh.password"] {
            let id = format!("sync-compare-diff-{side}-{path}");
            assert!(ui.visible(&id, cx), "{id} is highlighted");
        }
        let id = format!("sync-compare-same-{side}-address");
        assert!(ui.visible(&id, cx), "{id} is not highlighted");
        assert!(!ui.visible(&format!("sync-compare-{side}-deleted"), cx));
    }
    assert!(ui.visible("sync-keep-this", cx));
    assert!(ui.visible("sync-keep-other", cx));
    // What the dialog shows: the masked values, never the secrets.
    let items = ui.core(cx).sync_conflicts().unwrap();
    let rows = super::field_rows(&items[0]);
    let password = rows.iter().find(|row| row.path == "ssh.password").unwrap();
    assert_eq!(password.current.as_deref(), Some("••••••"));
    assert_eq!(password.other.as_deref(), Some("••••••"));
    let shown = format!("{rows:?}");
    assert!(!shown.contains(&current_secret) && !shown.contains(&other_secret));
    let port = rows.iter().find(|row| row.path == "ssh.port").unwrap();
    assert_eq!(
        (port.current.as_deref(), port.other.as_deref()),
        (Some("22"), Some("2222"))
    );

    ui.click("sync-keep-other", cx);
    assert!(!ui.visible("sync-compare", cx), "the dialog closed");
    assert!(ui.core(cx).sync_conflicts().unwrap().is_empty());
    assert_eq!(ui.label_of(host, cx).as_deref(), Some("web-2"));
    assert!(!ui.visible(&badge(host), cx), "the badge is gone");
    ui.finish(cx);
}

/// "Conflicts: N" in Account & Sync opens the list, named by object and
/// type; a row opens the comparison. The other version deleted the host:
/// its side says "Deleted". Closing leaves the conflict; "Keep this" keeps
/// the host and the list empties.
#[gpui::test]
fn list_opens_the_comparison_and_keep_this_keeps(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.host(&secret(), cx);
    ui.conflict(host, None, cx);
    ui.press("ctrl-,", cx);
    ui.click("settings-nav-account", cx);
    assert!(ui.visible("sync-conflicts", cx), "Conflicts: 1");
    ui.click("sync-conflicts", cx);
    assert!(ui.visible("sync-conflicts-list", cx));
    assert!(ui.visible_row(0, cx), "the conflict is listed");
    ui.click(("sync-conflict", 0usize), cx);
    assert!(ui.visible("sync-compare", cx));
    assert!(ui.visible("sync-compare-other-deleted", cx), "Deleted");
    assert!(!ui.visible("sync-compare-current-deleted", cx));
    // The deleted side has no fields; the live side's are all at stake.
    assert!(ui.visible("sync-compare-diff-current-address", cx));
    assert!(!ui.visible("sync-compare-diff-other-address", cx));

    // Closing without a choice leaves the conflict.
    ui.click("sync-compare-close", cx);
    assert!(!ui.visible("sync-compare", cx));
    assert_eq!(ui.core(cx).sync_conflicts().unwrap().len(), 1);
    assert!(ui.visible("sync-conflicts-list", cx), "back on the list");

    ui.click(("sync-conflict", 0usize), cx);
    ui.click("sync-keep-this", cx);
    assert!(!ui.visible("sync-compare", cx));
    assert!(ui.core(cx).sync_conflicts().unwrap().is_empty());
    assert_eq!(ui.label_of(host, cx).as_deref(), Some("web"), "kept");
    assert!(
        !ui.visible_row(0, cx),
        "the list under the comparison emptied"
    );
    ui.finish(cx);
}

impl Ui {
    fn visible_row(&self, index: usize, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(("sync-conflict", index))
                .is_some_and(|el| el.visible())
        })
        .unwrap()
    }
}

/// The generic comparison: an object without a card of its own (a
/// workspace) lists its fields the same way, and the list names its type.
#[test]
fn every_type_gets_the_field_list() {
    use shellpp_core::service::{ConflictItem, FieldDifference};
    let item = ConflictItem {
        conflict_id: Uuid::now_v7(),
        original_record_id: Uuid::now_v7(),
        original_type: "workspace".into(),
        title: "Deploy".into(),
        losing_deleted: false,
        losing_data: Some(map(json!({"name": "Deploy", "tabs": ["a", "b"]}))),
        losing_device_id: Some(Uuid::now_v7().to_string()),
        differences: vec![FieldDifference {
            path: "tabs".into(),
            current: "[\"a\"]".into(),
            other: "[\"a\",\"b\"]".into(),
        }],
        current_deleted: false,
        current_data: Some(map(json!({"name": "Deploy", "tabs": ["a"]}))),
        current_device_id: None,
        current_device_name: None,
        losing_device_name: Some("Laptop".into()),
        current_this_device: true,
        losing_this_device: false,
        current_changed_at: None,
        losing_changed_at: None,
    };
    assert_eq!(super::type_label(&item.original_type), "Workspace");
    let rows = super::field_rows(&item);
    assert_eq!(
        rows,
        vec![
            super::FieldRow {
                path: "name".into(),
                current: Some("Deploy".into()),
                other: Some("Deploy".into()),
                differs: false,
            },
            super::FieldRow {
                path: "tabs".into(),
                current: Some("a".into()),
                other: Some("a, b".into()),
                differs: true,
            },
        ]
    );
    assert_eq!(super::origin_text(&item, super::Side::Other), "Laptop");
    assert_eq!(
        super::origin_text(&item, super::Side::Current),
        super::strings::VERSION_THIS_DEVICE
    );
}
