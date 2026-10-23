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
use shellpp_core::service::{ConflictChoice, ConflictItem, KeepOther};
use uuid::Uuid;

use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
    /// The profile's generated master password (unlock after a lock).
    password: SecretString,
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
            .unwrap();
        let ui = Self {
            state,
            handle,
            _root: root,
            password,
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
        current_missing: false,
        current_data: Some(map(json!({"name": "Deploy", "tabs": ["a"]}))),
        keep_other: shellpp_core::service::KeepOther::Replaces,
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

/// An original still here that this device cannot read is shown as
/// "Missing"; "Keep other" is refused with the reason (the core reports
/// `KeepOther::Unreadable`).
#[test]
fn an_unreadable_current_version_explains_keep_other() {
    let item = ConflictItem {
        conflict_id: Uuid::now_v7(),
        original_record_id: Uuid::now_v7(),
        original_type: "known_host".into(),
        title: "known_host".into(),
        losing_deleted: false,
        losing_data: Some(map(json!({"comment": "edited elsewhere"}))),
        losing_device_id: Some(Uuid::now_v7().to_string()),
        differences: Vec::new(),
        current_deleted: false,
        current_missing: true,
        current_data: None,
        keep_other: KeepOther::Unreadable,
        current_device_id: None,
        current_device_name: None,
        losing_device_name: None,
        current_this_device: false,
        losing_this_device: false,
        current_changed_at: None,
        losing_changed_at: None,
    };
    assert!(!item.keep_other.allowed());
    assert_eq!(
        super::keep_other_note(&item),
        Some(super::strings::CURRENT_UNREADABLE)
    );
    assert_eq!(
        super::absent(&item, super::Side::Current),
        Some(("missing", super::strings::MISSING))
    );
}

impl Ui {
    /// Opens the comparison of `conflict` (as a row of the list does).
    fn open_compare(&self, conflict: Uuid, cx: &mut TestAppContext) -> ConflictItem {
        let item = self.item(conflict, cx);
        let (state, opened) = (self.state.clone(), item.clone());
        cx.update_window(self.handle, move |_, window, cx| {
            super::open(state, opened, None, window, cx);
        })
        .unwrap();
        self.settle(cx);
        assert!(self.visible("sync-compare", cx));
        item
    }

    fn item(&self, conflict: Uuid, cx: &mut TestAppContext) -> ConflictItem {
        self.core(cx)
            .sync_conflicts()
            .unwrap()
            .into_iter()
            .find(|item| item.conflict_id == conflict)
            .expect("the conflict is listed")
    }

    /// The current version's data as the comparison gets it, through a
    /// conflict copy resolved right away ("Keep this").
    fn current_data(&self, record: Uuid, cx: &mut TestAppContext) -> Map<String, Value> {
        let probe = self.conflict(record, None, cx);
        let data = self.item(probe, cx).current_data.unwrap();
        self.core(cx)
            .sync_conflict_resolve(probe, ConflictChoice::Current)
            .unwrap();
        self.settle(cx);
        data
    }

    fn known_host(&self, revoked: bool, cx: &mut TestAppContext) -> Uuid {
        let key: [u8; 32] = shellpp_core::crypto::random_array().unwrap();
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&key);
        let id = self
            .core(cx)
            .known_host_add(shellpp_core::service::dto::KnownHostAddInput {
                host: "192.0.2.30".into(),
                port: Some(22),
                key_type: "ssh-ed25519".into(),
                public_key: shellpp_core::known_hosts::encode_blob(&blob),
                revoked,
                comment: None,
            })
            .unwrap()
            .record_id;
        self.settle(cx);
        id
    }

    fn revoked(&self, known: Uuid, cx: &mut TestAppContext) -> bool {
        self.core(cx)
            .known_hosts_list()
            .unwrap()
            .into_iter()
            .find(|view| view.record_id == known)
            .expect("still a known host")
            .revoked
    }

    fn conflicts(&self, cx: &mut TestAppContext) -> usize {
        self.core(cx).sync_conflicts().unwrap().len()
    }
}

/// UI 13.6: a revoked known host key stays revoked. "Keep other" against
/// an edit made elsewhere says so under the button and keeps the key
/// revoked; against a deletion it is disabled with the reason, and a click
/// changes nothing. The list's hint no longer claims the newer change won.
#[gpui::test]
fn keep_other_says_a_revoked_key_stays_revoked(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let known = ui.known_host(true, cx);
    let mut edited = ui.current_data(known, cx);
    assert_eq!(edited["marker"], json!("revoked"));
    edited.remove("marker");
    edited.insert("comment".into(), json!("edited elsewhere"));
    let conflict = ui.conflict(known, Some(Value::Object(edited)), cx);
    let item = ui.open_compare(conflict, cx);
    assert_eq!(item.keep_other, KeepOther::KeepsRevocation);
    assert_eq!(
        super::keep_other_note(&item),
        Some(super::strings::KEEPS_REVOCATION)
    );
    assert!(ui.visible("sync-keep-other-note", cx), "the note says why");
    ui.click("sync-keep-other", cx);
    assert!(!ui.visible("sync-compare", cx), "resolved");
    assert_eq!(ui.conflicts(cx), 0);
    assert!(ui.revoked(known, cx), "the key stays revoked");

    let deletion = ui.conflict(known, None, cx);
    let item = ui.open_compare(deletion, cx);
    assert_eq!(item.keep_other, KeepOther::RevokedStays);
    assert_eq!(
        super::keep_other_note(&item),
        Some(super::strings::REVOKED_STAYS)
    );
    assert!(ui.visible("sync-keep-other-note", cx));
    ui.click("sync-keep-other", cx);
    assert!(ui.visible("sync-compare", cx), "disabled: nothing happens");
    assert_eq!(ui.conflicts(cx), 1, "the conflict stays");
    assert!(ui.revoked(known, cx));
    ui.click("sync-keep-this", cx);
    assert_eq!(ui.conflicts(cx), 0);
    assert!(!super::strings::CONFLICTS_HINT.contains("newer"));
    ui.finish(cx);
}

/// A plain host conflict: "Keep other" has no note and fields differ.
#[gpui::test]
fn keep_other_has_no_note_when_it_replaces(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.host(&secret(), cx);
    let conflict = ui.conflict(host, Some(json!({"address": "192.0.2.11"})), cx);
    let item = ui.open_compare(conflict, cx);
    assert_eq!(item.keep_other, KeepOther::Replaces);
    assert!(!ui.visible("sync-keep-other-note", cx));
    assert!(!ui.visible("sync-compare-same-fields", cx), "fields differ");
    ui.finish(cx);
}

/// Data-model 15: an original no longer here is "Missing", not "Deleted";
/// "Keep other" recreates it.
#[gpui::test]
fn a_missing_original_says_missing(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.host(&secret(), cx);
    let conflict = ui.conflict(
        host,
        Some(json!({"address": "192.0.2.10", "label": "web-2"})),
        cx,
    );
    let core = ui.core(cx);
    let vault_id = core.vaults().unwrap()[0].vault_id;
    // Not bound: the row goes at once; out of the trash, nothing is left.
    core.record_delete(shellpp_core::service::dto::RecordsRef {
        vault_id,
        record_ids: vec![host],
    })
    .unwrap();
    core.recently_deleted_delete(&[host]).unwrap();
    ui.settle(cx);
    ui.open_compare(conflict, cx);
    assert!(ui.visible("sync-compare-current-missing", cx), "Missing");
    assert!(
        !ui.visible("sync-compare-current-deleted", cx),
        "not Deleted"
    );
    assert!(!ui.visible("sync-compare-other-missing", cx));
    ui.click("sync-keep-other", cx);
    assert_eq!(ui.conflicts(cx), 0);
    assert_eq!(ui.label_of(host, cx).as_deref(), Some("web-2"), "recreated");
    ui.finish(cx);
}

/// UI 13.6: when the versions differ only where the dialog shows nothing
/// (the place in the list, an empty list or text against none), one line
/// says so and no field is marked.
#[gpui::test]
fn only_the_place_in_the_list_differs(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    // No secret: the data the comparison gets is the data itself.
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host(
                None,
                map(json!({"address": "192.0.2.12", "label": "db"})),
                cx,
            )
            .unwrap()
            .record_id
    });
    ui.settle(cx);
    let mut other = ui.current_data(host, cx);
    other.insert("order_key".into(), json!("zz"));
    other.insert("tags".into(), json!([]));
    other.insert("notes".into(), json!(""));
    let conflict = ui.conflict(host, Some(Value::Object(other)), cx);
    let item = ui.open_compare(conflict, cx);
    assert!(item.differences.is_empty(), "{:?}", item.differences);
    assert_eq!(
        super::same_fields_note(&item),
        Some(super::strings::ONLY_ORDER_DIFFERS)
    );
    assert!(
        ui.visible("sync-compare-same-fields", cx),
        "the line says so"
    );
    assert!(
        super::field_rows(&item).iter().all(|row| !row.differs),
        "nothing marked"
    );
    ui.finish(cx);
}

/// The badge (UI 13.6): a click opens the comparison and not the host's
/// panel; the badge goes with a lock and comes back at the unlock while the
/// conflict stays, and goes for good after "Keep this".
#[gpui::test]
fn the_badge_stays_off_the_card_and_goes_with_the_lock_and_keep_this(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.host(&secret(), cx);
    ui.conflict(
        host,
        Some(json!({"address": "192.0.2.10", "label": "web-2"})),
        cx,
    );
    assert!(ui.visible(&badge(host), cx));
    assert!(!ui.visible("host-panel", cx));
    ui.click(SharedString::from(badge(host)), cx);
    assert!(ui.visible("sync-compare", cx), "the comparison opened");
    assert!(!ui.visible("host-panel", cx), "the card's panel did not");
    ui.click("sync-compare-close", cx);
    assert!(!ui.visible("host-panel", cx));

    // A lock drops the conflicted ids with the keys; the unlock reads them
    // again.
    ui.press("ctrl-shift-l", cx);
    assert!(!ui.visible(&badge(host), cx), "locked: no badge");
    assert!(
        ui.state
            .read_with(cx, |state, _| state.sync_conflicted().is_empty()),
        "no conflicted id kept while locked"
    );
    let password = ui.password.clone();
    ui.state
        .update(cx, |state, cx| state.unlock(password, cx))
        .detach();
    ui.settle(cx);
    assert!(ui.visible(&badge(host), cx), "unlocked: the conflict stays");

    ui.click(SharedString::from(badge(host)), cx);
    ui.click("sync-keep-this", cx);
    assert!(!ui.visible("sync-compare", cx));
    assert!(
        !ui.visible(&badge(host), cx),
        "Keep this: the badge is gone"
    );
    assert_eq!(ui.label_of(host, cx).as_deref(), Some("web"));

    // The card itself opens its panel: the badge's click did not reach it.
    ui.click(crate::hosts::host_card_id(host), cx);
    assert!(ui.visible("host-panel", cx));
    ui.finish(cx);
}

/// A list row's badges go compact once the full ones would leave the title
/// less than `BADGE_TITLE_MIN`: 12 px after the title, 8 px between badges.
#[test]
fn badges_go_compact_before_the_title_gets_too_short() {
    use super::{BADGE_TITLE_MIN, compact_badges};
    assert!(!compact_badges(0., &[]));
    let full = [60., 70.];
    let fits = BADGE_TITLE_MIN + 12. + 60. + 8. + 70.;
    assert!(!compact_badges(fits, &full));
    assert!(compact_badges(fits - 1., &full));
    assert!(!compact_badges(BADGE_TITLE_MIN + 12. + 70., &full[1..]));
}
