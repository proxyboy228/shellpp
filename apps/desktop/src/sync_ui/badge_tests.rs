//! Headless tests of the conflict badge on the cards of every list (UI 13.6,
//! data model section 15) other than hosts (`compare/tests.rs`): groups,
//! keys, identities, forwarding rules, snippets and their packages, known
//! hosts. A conflict copy is written through the core's test hook (no sync
//! server); temporary profiles only, keys generated at run time.

use std::time::{Duration, SystemTime};

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, SharedString, TestAppContext,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::service::snippets::RecordInput;
use uuid::Uuid;

use crate::app::{self, RootView};
use crate::shell::Shell;
use crate::state::AppState;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<RootView>,
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
            // Dialogs animate on the real clock: with motion on, a button's
            // position depends on timing and a click can miss it under load.
            cx.set_reduce_motion(true);
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
            root,
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

    fn visible(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
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

    fn vault(&self, cx: &mut TestAppContext) -> Uuid {
        self.state
            .read_with(cx, |state, _| state.vault_id().unwrap())
    }

    fn shell(&self, cx: &mut TestAppContext) -> Entity<Shell> {
        self.root
            .read_with(cx, |root, _| root.shell().cloned().unwrap())
    }

    /// Sync kept `losing` (`None`: a deletion) of `record` from another
    /// device as a conflict copy.
    fn conflict(&self, record: Uuid, losing: Option<Value>, cx: &mut TestAppContext) {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        self.core(cx)
            .sync_conflict_insert_for_test(record, losing.map(map), Uuid::now_v7(), at)
            .unwrap();
        self.settle(cx);
    }

    fn conflicts(&self, cx: &mut TestAppContext) -> usize {
        self.core(cx).sync_conflicts().unwrap().len()
    }

    /// A click on `badge` opens the comparison and leaves the card alone
    /// (`untouched`: no selection, no panel, no open folder); closing the
    /// comparison keeps the conflict and the badge.
    fn badge_opens_the_comparison(
        &self,
        badge: SharedString,
        untouched: impl Fn(&Self, &mut TestAppContext) -> bool,
        cx: &mut TestAppContext,
    ) {
        assert!(self.visible(badge.clone(), cx), "{badge} is shown");
        self.click(badge.clone(), cx);
        assert!(self.visible("sync-compare", cx), "{badge} opened it");
        assert!(untouched(self, cx), "{badge} did not reach the card");
        self.click("sync-compare-close", cx);
        assert!(!self.visible("sync-compare", cx));
        assert!(self.visible(badge.clone(), cx), "{badge} stays");
    }

    /// "Keep this" from the badge's comparison resolves the conflict: the
    /// badge goes away.
    fn keep_this(&self, badge: SharedString, cx: &mut TestAppContext) {
        let before = self.conflicts(cx);
        self.click(badge.clone(), cx);
        assert!(self.visible("sync-compare", cx));
        self.click("sync-keep-this", cx);
        assert!(!self.visible("sync-compare", cx), "the dialog closed");
        assert_eq!(self.conflicts(cx), before - 1);
        assert!(!self.visible(badge.clone(), cx), "{badge} is gone");
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// Groups on the Hosts screen, in both layouts: the badge does not open
/// the group nor select it.
#[gpui::test]
fn group_cards_show_the_badge(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let group = ui.state.update(cx, |state, cx| {
        state.create_group_named("lab", None, cx).unwrap().record_id
    });
    ui.settle(cx);
    let badge = crate::hosts::group_conflict_badge_id(group);
    assert!(!ui.visible(badge.clone(), cx), "no conflict yet");
    ui.conflict(group, Some(json!({"name": "lab-2"})), cx);
    let untouched = |ui: &Ui, cx: &mut TestAppContext| {
        ui.state.read_with(cx, |state, _| {
            state.location().is_none() && state.selection().is_empty()
        })
    };
    ui.badge_opens_the_comparison(badge.clone(), untouched, cx);
    let hosts = ui
        .shell(cx)
        .read_with(cx, |shell, _| shell.hosts_screen().clone());
    hosts.update(cx, |screen, cx| {
        screen.set_layout(crate::state::hosts_view::HostsLayout::List, cx)
    });
    ui.settle(cx);
    ui.badge_opens_the_comparison(badge.clone(), untouched, cx);
    ui.keep_this(badge, cx);
    ui.finish(cx);
}

/// Keys and identities on the Keychain screen, in both layouts: the
/// badge of a key does not open its panel.
#[gpui::test]
fn keychain_cards_show_the_badge(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::KeyGenerateInput;
    use shellpp_core::service::keychain::IdentitySaveInput;

    let ui = Ui::open(cx);
    let (core, vault_id) = (ui.core(cx), ui.vault(cx));
    let key = core
        .key_generate(KeyGenerateInput {
            vault_id,
            label: "deploy".into(),
            kind: crate::state::records_edit::KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key
        .record_id;
    let identity = core
        .identity_save(IdentitySaveInput {
            vault_id,
            record_id: None,
            label: "ops".into(),
            username: "ops".into(),
            key_id: None,
            password: crate::state::records_edit::PasswordChange::Keep,
        })
        .unwrap()
        .record_id;
    ui.click("nav-keychain", cx);
    let (key_badge, identity_badge) = (
        crate::keychain::key_conflict_badge_id(key),
        crate::keychain::identity_conflict_badge_id(identity),
    );
    assert!(ui.visible(crate::keychain::key_card_id(key), cx));
    assert!(!ui.visible(key_badge.clone(), cx), "no conflict yet");
    assert!(!ui.visible(identity_badge.clone(), cx), "no conflict yet");
    ui.conflict(key, Some(json!({"label": "deploy-2"})), cx);
    ui.conflict(
        identity,
        Some(json!({"label": "ops-2", "username": "root"})),
        cx,
    );
    let screen = ui
        .shell(cx)
        .read_with(cx, |shell, _| shell.keychain_screen().clone());
    let untouched = {
        let screen = screen.clone();
        move |_: &Ui, cx: &mut TestAppContext| {
            screen.read_with(cx, |screen, _| {
                screen.selected().is_none() && !screen.has_panel()
            })
        }
    };
    ui.badge_opens_the_comparison(key_badge.clone(), &untouched, cx);
    ui.badge_opens_the_comparison(identity_badge.clone(), &untouched, cx);
    screen.update(cx, |screen, cx| {
        screen.set_layout(crate::keychain::KeychainLayout::List, cx)
    });
    ui.settle(cx);
    ui.badge_opens_the_comparison(key_badge.clone(), &untouched, cx);
    ui.badge_opens_the_comparison(identity_badge.clone(), &untouched, cx);
    ui.keep_this(key_badge, cx);
    ui.keep_this(identity_badge, cx);
    ui.finish(cx);
}

/// Forwarding rules: the badge does not select the rule (its panel).
#[gpui::test]
fn forwarding_cards_show_the_badge(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.state.update(cx, |state, cx| {
        state
            .save_host_with_secrets(
                None,
                map(json!({"address": "192.0.2.10", "label": "web"})),
                Default::default(),
                cx,
            )
            .unwrap()
            .record_id
    });
    let vault_id = ui.vault(cx);
    let rule = ui
        .core(cx)
        .forwarding_rule_save(RecordInput {
            vault_id,
            record_id: None,
            data: map(
                json!({"type": "local", "host_id": host, "label": "Database",
                "bind_port": 15432, "dest_address": "db.internal", "dest_port": 5432}),
            ),
        })
        .unwrap()
        .record
        .record_id;
    ui.click("nav-port-forwarding", cx);
    let badge = crate::forwarding::conflict_badge_id(rule);
    assert!(ui.visible(crate::forwarding::card_id(rule), cx));
    assert!(!ui.visible(badge.clone(), cx), "no conflict yet");
    ui.conflict(rule, None, cx);
    let screen = ui
        .shell(cx)
        .read_with(cx, |shell, _| shell.forwarding_screen().clone());
    ui.badge_opens_the_comparison(
        badge.clone(),
        move |_, cx| screen.read_with(cx, |screen, _| screen.selected().is_none()),
        cx,
    );
    ui.keep_this(badge, cx);
    ui.finish(cx);
}

/// Snippets and their packages: the badge does not select the snippet
/// nor open the package.
#[gpui::test]
fn snippet_cards_show_the_badge(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (core, vault_id) = (ui.core(cx), ui.vault(cx));
    let snippet = core
        .snippet_save(RecordInput {
            vault_id,
            record_id: None,
            data: map(json!({"label": "Uptime", "script": "uptime"})),
        })
        .unwrap()
        .record
        .record_id;
    let package = core
        .snippet_package_save(RecordInput {
            vault_id,
            record_id: None,
            data: map(json!({"name": "Ops"})),
        })
        .unwrap()
        .record
        .record_id;
    ui.click("nav-snippets", cx);
    let (snippet_badge, package_badge) = (
        crate::snippets::conflict_badge_id(snippet),
        crate::snippets::package_conflict_badge_id(package),
    );
    assert!(ui.visible(crate::snippets::card_id(snippet), cx));
    assert!(ui.visible(crate::snippets::package_id(package), cx));
    assert!(!ui.visible(snippet_badge.clone(), cx), "no conflict yet");
    assert!(!ui.visible(package_badge.clone(), cx), "no conflict yet");
    ui.conflict(
        snippet,
        Some(json!({"label": "Uptime", "script": "uptime -p"})),
        cx,
    );
    ui.conflict(package, Some(json!({"name": "Ops-2"})), cx);
    let screen = ui
        .shell(cx)
        .read_with(cx, |shell, _| shell.snippets_screen().clone());
    let untouched = move |_: &Ui, cx: &mut TestAppContext| {
        screen.read_with(cx, |screen, _| {
            screen.selected().is_none() && screen.open_package().is_none()
        })
    };
    ui.badge_opens_the_comparison(snippet_badge.clone(), &untouched, cx);
    ui.badge_opens_the_comparison(package_badge.clone(), &untouched, cx);
    ui.keep_this(snippet_badge, cx);
    ui.keep_this(package_badge, cx);
    ui.finish(cx);
}

/// Known hosts, in both layouts: the badge does not select the entry (its
/// details panel).
#[gpui::test]
fn known_host_cards_show_the_badge(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::KnownHostAddInput;

    let ui = Ui::open(cx);
    let key: [u8; 32] = shellpp_core::crypto::random_array().unwrap();
    let mut blob = Vec::new();
    blob.extend_from_slice(&11u32.to_be_bytes());
    blob.extend_from_slice(b"ssh-ed25519");
    blob.extend_from_slice(&32u32.to_be_bytes());
    blob.extend_from_slice(&key);
    let entry = ui
        .core(cx)
        .known_host_add(KnownHostAddInput {
            host: "web.example.com".into(),
            port: None,
            key_type: "ssh-ed25519".into(),
            public_key: shellpp_core::known_hosts::encode_blob(&blob),
            revoked: false,
            comment: None,
        })
        .unwrap()
        .record_id;
    ui.click("nav-known-hosts", cx);
    let badge = crate::known_hosts::conflict_badge_id(entry);
    assert!(ui.visible(crate::known_hosts::card_id(entry), cx));
    assert!(!ui.visible(badge.clone(), cx), "no conflict yet");
    ui.conflict(entry, None, cx);
    let screen = ui
        .shell(cx)
        .read_with(cx, |shell, _| shell.known_hosts_screen().clone());
    let untouched = {
        let screen = screen.clone();
        move |_: &Ui, cx: &mut TestAppContext| {
            screen.read_with(cx, |screen, _| {
                screen.selected().is_none() && !screen.has_panel()
            })
        }
    };
    ui.badge_opens_the_comparison(badge.clone(), &untouched, cx);
    screen.update(cx, |screen, cx| {
        screen.set_layout(crate::known_hosts::KnownHostsLayout::List, cx)
    });
    ui.settle(cx);
    ui.badge_opens_the_comparison(badge.clone(), &untouched, cx);
    ui.keep_this(badge, cx);
    ui.finish(cx);
}
