//! Headless tests of the Keychain screen (GPUI `TestAppContext`, no real
//! window): the screen is the window's root view, or the whole app for the
//! sidebar and the section switch. Every test runs on its own temporary
//! profile with the in-memory platform (secret store, clipboard and clock:
//! never the OS keychain or clipboard, never the permanent profile). Keys
//! and passphrases are generated at run time; the `~/.ssh` folder of
//! "Import from ~/.ssh" is always a temporary folder (never the real one).

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, ExternalPaths, FileDropEvent,
    InputEvent as _, TestAppContext, Window, WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::SecretString;
use shellpp_core::keys::convert::KeyProblem;
use shellpp_core::keys::generate::generate;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::dto::{KeyGenerateInput, KeyImportInput};
use uuid::Uuid;

use super::*;
use crate::state::AppState;
use crate::state::records_edit::{KeyKind, PasswordChange};
use crate::testing::TestProfile;

/// A passphrase generated at run time.
fn new_passphrase() -> String {
    format!("pp-{}", crate::profile::random_suffix().expect("random"))
}

/// The OpenSSH text of a new key (encrypted with `passphrase` if given).
fn key_text(kind: KeyKind, comment: &str, passphrase: Option<&str>) -> String {
    let passphrase = passphrase.map(SecretString::from);
    generate(kind, comment, passphrase.as_ref())
        .expect("key")
        .to_string()
}

struct Ui {
    state: Entity<AppState>,
    screen: Entity<KeychainScreen>,
    handle: AnyWindowHandle,
    memory: MemoryPlatform,
    /// Temporary files of this test (the `~/.ssh` folder, dropped files).
    dir: tempfile::TempDir,
    /// Owns the profile folder; dropped after the app state shut down.
    profile: Option<TestProfile>,
}

fn install(cx: &mut TestAppContext) {
    cx.update(crate::testing::init_ui);
}

impl Ui {
    /// A 1280 x 1000 window with the screen over a temporary profile; `seed`
    /// writes records through the core first.
    fn open(cx: &mut TestAppContext, seed: impl FnOnce(&shellpp_core::Core, Uuid)) -> Self {
        install(cx);
        let dir = tempfile::tempdir().expect("temp dir");
        let ssh = dir.path().join("ssh");
        std::fs::create_dir(&ssh).unwrap();
        // Never the real ~/.ssh.
        cx.update(|cx| set_ssh_folder(Some(ssh), cx));
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let memory = profile.memory().expect("memory platform").clone();
        let vault_id = opened.core.vaults().unwrap()[0].vault_id;
        seed(&opened.core, vault_id);
        let state = cx.new(|cx| AppState::new(opened, cx));
        cx.run_until_parked();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                // Tall enough for every block of the forms, so the
                // clicks reach them without scrolling.
                size(px(1280.), px(1000.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| KeychainScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let ui = Self {
            state,
            screen,
            handle,
            memory,
            dir,
            profile: Some(profile),
        };
        ui.frame(cx, |_, _| {});
        ui.frame(cx, |_, _| {});
        ui
    }

    fn frame<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
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

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    /// Clicks into the input `id` and types `text` at its end.
    fn type_into(&self, cx: &mut TestAppContext, id: &'static str, text: &str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("end", cx);
            window.input(text, cx);
        });
    }

    fn clear(&self, cx: &mut TestAppContext, id: &'static str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
        });
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn keys(&self, cx: &mut TestAppContext) -> Vec<KeyView> {
        self.screen
            .read_with(cx, |screen, _| screen.keys().to_vec())
    }

    fn key_by_label(&self, cx: &mut TestAppContext, label: &str) -> KeyView {
        self.keys(cx)
            .into_iter()
            .find(|key| key.label == label)
            .unwrap_or_else(|| panic!("a key labelled {label}"))
    }

    fn panel(&self, cx: &mut TestAppContext) -> Entity<KeyPanel> {
        self.screen
            .read_with(cx, |screen, _| screen.key_panel().cloned())
            .expect("the key panel is open")
    }

    fn generate_form(&self, cx: &mut TestAppContext) -> Entity<GenerateForm> {
        self.screen
            .read_with(cx, |screen, _| screen.generate_form().cloned())
            .expect("the generate form is open")
    }

    fn import_form(&self, cx: &mut TestAppContext) -> Entity<ImportForm> {
        self.screen
            .read_with(cx, |screen, _| screen.import_form().cloned())
            .expect("the import form is open")
    }

    /// Puts `text` into the import form as a paste would.
    fn paste_key(&self, cx: &mut TestAppContext, text: &str) {
        let form = self.import_form(cx);
        self.frame(cx, |window, cx| {
            form.update(cx, |form, cx| form.set_text(text, window, cx))
        });
    }

    fn finish(mut self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
        drop(self.profile.take());
    }
}

/// Imports `text` through the core (seeding).
fn import(core: &shellpp_core::Core, vault_id: Uuid, label: &str, text: &str) -> KeyView {
    core.key_import(KeyImportInput {
        vault_id,
        label: Some(label.to_owned()),
        private_key: SecretString::from(text.to_owned()),
        passphrase: None,
        save_passphrase: false,
    })
    .expect("imported")
    .key
}

/// The seed of most tests: an Ed25519 and an ECDSA P-256 key, an
/// encrypted Ed25519 key with its passphrase saved, and four identities
/// (password, key, both, nothing).
fn sample(core: &shellpp_core::Core, vault_id: Uuid) {
    let deploy = import(
        core,
        vault_id,
        "Deploy",
        &key_text(KeyKind::Ed25519, "deploy@test", None),
    );
    import(
        core,
        vault_id,
        "CI runner",
        &key_text(KeyKind::EcdsaP256, "ci@test", None),
    );
    core.key_generate(KeyGenerateInput {
        vault_id,
        label: "Laptop".into(),
        kind: KeyKind::Ed25519,
        passphrase: Some(SecretString::from(new_passphrase())),
        save_passphrase: true,
    })
    .expect("generated");
    for (label, password, key) in [
        ("Ops password", true, false),
        ("Ops key", false, true),
        ("Ops both", true, true),
        ("Nothing set", false, false),
    ] {
        core.identity_save(shellpp_core::service::keychain::IdentitySaveInput {
            vault_id,
            record_id: None,
            label: label.into(),
            username: "ops".into(),
            key_id: key.then_some(deploy.record_id),
            password: if password {
                PasswordChange::Set(SecretString::from(new_passphrase()))
            } else {
                PasswordChange::Keep
            },
        })
        .expect("identity");
    }
}

// -- Content (7.2) ----------------------------------------------------------------------

#[gpui_kit::test]
fn keys_and_identities_show_as_cards(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let keys = ui.keys(cx);
    assert_eq!(keys.len(), 3);
    assert!(ui.visible(cx, "kc-heading-keys"));
    assert!(ui.visible(cx, "kc-heading-identities"));
    for key in &keys {
        assert!(ui.visible(cx, key_card_id(key.record_id)));
        assert!(ui.visible(cx, key_subtitle_id(key.record_id)));
        assert!(!ui.visible(cx, key_unknown_id(key.record_id)));
    }
    let deploy = ui.key_by_label(cx, "Deploy");
    let ci = ui.key_by_label(cx, "CI runner");
    assert_eq!(key_subtitle(&deploy), "Type ED25519");
    assert_eq!(key_subtitle(&ci), "Type ECDSA P-256");
    // Identities: the method as the subtitle.
    let identities = ui
        .screen
        .read_with(cx, |screen, _| screen.identities().to_vec());
    assert_eq!(identities.len(), 4);
    let method =
        |label: &str| identity_method(identities.iter().find(|view| view.label == label).unwrap());
    assert_eq!(method("Ops password"), "Password");
    assert_eq!(method("Ops key"), "Key");
    assert_eq!(method("Ops both"), "Password and key");
    assert_eq!(method("Nothing set"), "No authentication method");
    for identity in &identities {
        assert!(ui.visible(cx, identity_card_id(identity.record_id)));
        assert!(ui.visible(cx, identity_subtitle_id(identity.record_id)));
    }
    // Identity cards are read-only: a click opens nothing.
    ui.click(cx, identity_card_id(identities[0].record_id));
    assert!(!ui.visible(cx, "kc-panel"));
    // The action bar: New key with its menu; Certificate and FIDO2 are there.
    for id in [
        "kc-new-key",
        "kc-new-key-menu",
        "kc-certificate",
        "kc-fido2",
        "kc-search",
        "kc-view-tiles",
        "kc-view-list",
        "kc-sort",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    ui.finish(cx);
}

/// A key this client cannot read shows the type "unknown" with a warning
/// icon on its card, and the reason in its panel (UI 7.2).
#[gpui_kit::test]
fn an_unreadable_key_shows_unknown_with_a_warning(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let mut unknown = ui.key_by_label(cx, "Deploy");
    unknown.algorithm = "unknown".into();
    unknown.fingerprint = None;
    let id = unknown.record_id;
    assert_eq!(key_subtitle(&unknown), "Type unknown");
    assert_eq!(key_type("ssh-dss"), None);
    assert_eq!(key_type("ssh-rsa"), Some("RSA"));
    // As a record synced from a newer client would arrive.
    ui.screen.update(cx, |screen, cx| {
        let key = screen
            .keys
            .iter_mut()
            .find(|key| key.record_id == id)
            .unwrap();
        *key = unknown;
        screen.refilter(cx);
    });
    assert!(ui.visible(cx, key_unknown_id(id)));
    ui.click(cx, key_card_id(id));
    assert!(ui.visible(cx, "kc-key-type-unknown"));
    assert!(ui.visible(cx, "kc-key-unknown-reason"));
    assert!(!ui.visible(cx, "kc-key-fingerprint"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn search_sort_and_the_list_are_remembered(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let deploy = ui.key_by_label(cx, "Deploy");
    ui.type_into(cx, "kc-search", "runner");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_key_ids()),
        vec![ui.key_by_label(cx, "CI runner").record_id]
    );
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.shown_identity_ids())
            .is_empty()
    );
    // The fingerprint is searchable too, and identities by username.
    ui.clear(cx, "kc-search");
    ui.type_into(cx, "kc-search", &deploy.fingerprint.clone().unwrap()[7..20]);
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_key_ids()),
        vec![deploy.record_id]
    );
    ui.clear(cx, "kc-search");
    ui.type_into(cx, "kc-search", "nothing like this");
    assert!(ui.visible(cx, "kc-no-matches"));
    ui.clear(cx, "kc-search");
    ui.type_into(cx, "kc-search", "OPS");
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.shown_identity_ids())
            .len(),
        4
    );
    ui.clear(cx, "kc-search");

    // Sorting: by name (A to Z), by creation date (newest first).
    let names = |ui: &Ui, cx: &mut TestAppContext| -> Vec<String> {
        ui.screen.read_with(cx, |screen, _| {
            screen
                .shown_key_ids()
                .iter()
                .map(|id| screen.key(*id).unwrap().label.clone())
                .collect()
        })
    };
    assert_eq!(names(&ui, cx), ["CI runner", "Deploy", "Laptop"]);
    ui.click(cx, "kc-sort");
    assert!(ui.visible(cx, "kc-sort-created"));
    ui.click(cx, "kc-sort-created");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.sort()),
        KeychainSort::Created
    );
    assert_eq!(names(&ui, cx), ["Laptop", "CI runner", "Deploy"]);

    // The list: column names over rows with the same ids.
    ui.click(cx, "kc-view-list");
    assert!(ui.visible(cx, "kc-keys-columns"));
    assert!(ui.visible(cx, "kc-identities-columns"));
    assert!(ui.visible(cx, key_card_id(deploy.record_id)));
    // Both are remembered: a new screen opens with the list, by date.
    let state = ui.state.clone();
    let (handle, again) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| KeychainScreen::new(state, window, cx))
            })
        })
        .unwrap();
    assert_eq!(
        again.read_with(cx, |screen, _| (screen.layout(), screen.sort())),
        (KeychainLayout::List, KeychainSort::Created)
    );
    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    ui.click(cx, "kc-view-tiles");
    assert!(!ui.visible(cx, "kc-keys-columns"));
    ui.finish(cx);
}

// -- The key panel (7.3) ----------------------------------------------------------------

/// Mandatory: the private key is hidden by default; the panel shows the
/// type, the SHA256 fingerprint and a note instead. Show reveals it, Hide
/// drops it.
#[gpui_kit::test]
fn the_private_key_is_hidden_until_show(cx: &mut TestAppContext) {
    let text = key_text(KeyKind::Ed25519, "hidden@test", None);
    let seeded = text.clone();
    let ui = Ui::open(cx, move |core, vault| {
        import(core, vault, "Hidden", &seeded);
    });
    let key = ui.key_by_label(cx, "Hidden");
    ui.click(cx, key_card_id(key.record_id));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(key.record_id)
    );
    for id in [
        "kc-panel",
        "kc-key-panel",
        "kc-key-vault",
        "kc-key-label",
        "kc-key-type",
        "kc-key-fingerprint",
        "kc-key-hidden-note",
        "kc-key-show",
        "kc-key-copy-private",
        "kc-key-public",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    // Key export, at the bottom of the panel (shown disabled, D33).
    ui.frame(cx, |window, cx| {
        window.scroll(
            "kc-key-panel-body",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-2000.))),
            cx,
        )
    });
    assert!(ui.visible(cx, "kc-key-export"));
    assert!(!ui.visible(cx, "kc-key-private"));
    let panel = ui.panel(cx);
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    assert!(
        panel
            .read_with(cx, |panel, _| panel.revealed_text())
            .is_none()
    );
    assert!(key.fingerprint.as_deref().unwrap().starts_with("SHA256:"));

    ui.click(cx, "kc-key-show");
    assert!(ui.visible(cx, "kc-key-private"));
    assert!(!ui.visible(cx, "kc-key-hidden-note"));
    assert_eq!(
        panel
            .read_with(cx, |panel, _| panel.revealed_text())
            .unwrap()
            .as_ref(),
        text.trim()
    );
    ui.click(cx, "kc-key-hide");
    assert!(!ui.visible(cx, "kc-key-private"));
    assert!(ui.visible(cx, "kc-key-hidden-note"));
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    ui.finish(cx);
}

/// Mandatory: leaving the panel hides the key again: closing it, showing
/// another key, Esc, and a lock.
#[gpui_kit::test]
fn the_private_key_is_hidden_again_on_leaving_the_panel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let deploy = ui.key_by_label(cx, "Deploy");
    let ci = ui.key_by_label(cx, "CI runner");
    let reveal = |ui: &Ui, cx: &mut TestAppContext, id: Uuid| -> Entity<KeyPanel> {
        ui.click(cx, key_card_id(id));
        ui.click(cx, "kc-key-show");
        let panel = ui.panel(cx);
        assert!(panel.read_with(cx, |panel, _| panel.is_revealed()));
        panel
    };

    // The collapse button.
    let panel = reveal(&ui, cx, deploy.record_id);
    ui.click(cx, "panel-close");
    assert!(!ui.visible(cx, "kc-panel"));
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    ui.click(cx, key_card_id(deploy.record_id));
    assert!(ui.visible(cx, "kc-key-hidden-note"));
    assert!(!ui.visible(cx, "kc-key-private"));

    // Another key.
    let panel = reveal(&ui, cx, deploy.record_id);
    ui.click(cx, key_card_id(ci.record_id));
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    assert!(!ui.visible(cx, "kc-key-private"));
    ui.click(cx, key_card_id(deploy.record_id));
    assert!(!ui.visible(cx, "kc-key-private"));

    // Esc on the cards.
    let panel = reveal(&ui, cx, deploy.record_id);
    ui.click(cx, key_card_id(deploy.record_id));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert!(!ui.visible(cx, "kc-panel"));
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));

    // The window shows something else (the shell calls this every frame).
    let panel = reveal(&ui, cx, deploy.record_id);
    ui.screen
        .update(cx, |screen, cx| screen.set_shown(false, cx));
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));

    // A lock drops the panel with everything in it.
    let panel = reveal(&ui, cx, deploy.record_id);
    ui.state.update(cx, |state, cx| state.lock(cx)).unwrap();
    cx.run_until_parked();
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    assert!(!ui.visible(cx, "kc-panel"));
    assert!(ui.keys(cx).is_empty());
    ui.finish(cx);
}

/// Mandatory: Copy private key goes through the core's sensitive copy and
/// is cleared after 30 seconds while the clipboard still holds it; it never
/// reaches the UI's own clipboard.
#[gpui_kit::test]
fn copy_private_key_is_cleared_after_thirty_seconds(cx: &mut TestAppContext) {
    let text = key_text(KeyKind::Ed25519, "copy@test", None);
    let seeded = text.clone();
    let ui = Ui::open(cx, move |core, vault| {
        import(core, vault, "Copied", &seeded);
    });
    let key = ui.key_by_label(cx, "Copied");
    ui.click(cx, key_card_id(key.record_id));
    ui.click(cx, "kc-key-copy-private");
    let clipboard = ui.memory.clipboard.clone();
    assert_eq!(clipboard.text().as_deref(), Some(text.trim()));
    assert!(clipboard.last_was_sensitive());
    // Not shown by copying, and not in GPUI's clipboard.
    assert!(!ui.visible(cx, "kc-key-private"));
    assert_ne!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some(text.trim())
    );
    let core = ui.core(cx);
    ui.memory.clock.advance(Duration::from_secs(29));
    core.tick();
    assert!(clipboard.text().is_some(), "still there before 30 seconds");
    ui.memory.clock.advance(Duration::from_secs(2));
    core.tick();
    assert_eq!(clipboard.text(), None, "cleared after 30 seconds");

    // Something the user copied meanwhile stays.
    ui.click(cx, "kc-key-copy-private");
    clipboard.set_text(Some("user text"));
    ui.memory.clock.advance(Duration::from_secs(31));
    core.tick();
    assert_eq!(clipboard.text().as_deref(), Some("user text"));
    drop(core);
    ui.finish(cx);
}

/// The public key is shown (copied as ordinary text, it is not a secret);
/// an unencrypted key carries the hint, an encrypted one says where its
/// passphrase is.
#[gpui_kit::test]
fn public_key_and_passphrase_hints(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let deploy = ui.key_by_label(cx, "Deploy");
    ui.click(cx, key_card_id(deploy.record_id));
    let public = ui
        .panel(cx)
        .read_with(cx, |panel, _| panel.public_key().map(str::to_owned))
        .unwrap();
    assert!(public.starts_with("ssh-ed25519 "));
    ui.click(cx, "kc-key-copy-public");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(public.clone())
    );
    assert!(ui.visible(cx, "kc-key-no-passphrase"));
    assert!(!ui.visible(cx, "kc-key-passphrase-note"));
    // The "…" menu copies it too; Duplicate and Export to file are there.
    ui.click(cx, "panel-menu");
    for id in [
        "kc-menu-copy-public",
        "kc-menu-duplicate",
        "kc-menu-export-file",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    ui.frame(cx, |window, cx| window.press("escape", cx));

    let laptop = ui.key_by_label(cx, "Laptop");
    assert!(laptop.encrypted && laptop.passphrase_saved);
    ui.click(cx, key_card_id(laptop.record_id));
    assert!(ui.visible(cx, "kc-key-passphrase-note"));
    assert!(!ui.visible(cx, "kc-key-no-passphrase"));
    ui.finish(cx);
}

/// Remove (D50): the confirmation names who uses the key and warns; Cancel
/// keeps it; Remove takes it out, and Undo brings it back.
#[gpui_kit::test]
fn remove_names_the_users_of_the_key_and_can_be_undone(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |core, vault_id| {
        sample(core, vault_id);
        let deploy = core
            .keys_list()
            .unwrap()
            .into_iter()
            .find(|key| key.label == "Deploy")
            .unwrap();
        core.record_save(shellpp_core::service::dto::RecordSaveInput {
            vault_id,
            record_id: None,
            record_type: shellpp_core::service::dto::RecordType::Host,
            data: serde_json::json!({
                "label": "web",
                "address": "web.example.com",
                "ssh": {"key_id": deploy.record_id.to_string()}
            })
            .as_object()
            .cloned()
            .unwrap(),
            secrets: Default::default(),
        })
        .expect("host");
    });
    let deploy = ui.key_by_label(cx, "Deploy");
    ui.click(cx, key_card_id(deploy.record_id));
    ui.click(cx, "panel-menu");
    ui.click(cx, "kc-menu-remove");
    assert!(ui.visible(cx, "kc-remove-dialog"));
    assert!(ui.visible(cx, "kc-remove-warning"));
    assert!(ui.visible(cx, "kc-remove-hosts"));
    assert!(ui.visible(cx, "kc-remove-identities"));
    assert!(!ui.visible(cx, "kc-remove-groups"));
    ui.click(cx, "kc-remove-cancel");
    assert!(!ui.visible(cx, "kc-remove-dialog"));
    assert_eq!(ui.keys(cx).len(), 3, "Cancel keeps the key");

    ui.click(cx, "panel-menu");
    ui.click(cx, "kc-menu-remove");
    ui.click(cx, "kc-remove-confirm");
    let labels: Vec<String> = ui.keys(cx).into_iter().map(|key| key.label).collect();
    assert!(!labels.contains(&"Deploy".to_owned()), "{labels:?}");
    assert!(!ui.visible(cx, "kc-key-panel"), "its panel closed");
    // Undo (the toast's action) brings it back under its id.
    ui.state
        .update(cx, |state, cx| {
            state.restore_records(vec![deploy.record_id], cx)
        })
        .expect("undo");
    ui.frame(cx, |_, _| {});
    ui.frame(cx, |_, _| {});
    assert_eq!(ui.key_by_label(cx, "Deploy").record_id, deploy.record_id);

    // A key nobody uses says so.
    let laptop = ui.key_by_label(cx, "Laptop");
    ui.click(cx, key_card_id(laptop.record_id));
    ui.click(cx, "panel-menu");
    ui.click(cx, "kc-menu-remove");
    assert!(ui.visible(cx, "kc-remove-dialog"));
    assert!(!ui.visible(cx, "kc-remove-warning"));
    ui.click(cx, "kc-remove-cancel");
    ui.finish(cx);
}

/// The label saves after the autosave delay (UI 1.3); an empty one is
/// outlined with its reason and not saved.
#[gpui_kit::test]
fn the_label_is_saved_as_in_the_saving_rule(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let deploy = ui.key_by_label(cx, "Deploy");
    ui.click(cx, key_card_id(deploy.record_id));
    ui.type_into(cx, "kc-key-label", " (prod)");
    let panel = ui.panel(cx);
    assert!(panel.read_with(cx, |panel, _| panel.is_saving()));
    cx.executor()
        .advance_clock(crate::theme::metrics::AUTOSAVE_DEBOUNCE + Duration::from_millis(50));
    cx.run_until_parked();
    let saved = |ui: &Ui, cx: &mut TestAppContext| {
        ui.core(cx)
            .keys_list()
            .unwrap()
            .into_iter()
            .find(|key| key.record_id == deploy.record_id)
            .unwrap()
            .label
    };
    assert_eq!(saved(&ui, cx), "Deploy (prod)");
    assert!(ui.visible(cx, key_card_id(deploy.record_id)));
    assert_eq!(
        ui.key_by_label(cx, "Deploy (prod)").record_id,
        deploy.record_id
    );

    // Empty: refused with its reason, the last valid label stays.
    ui.clear(cx, "kc-key-label");
    cx.executor()
        .advance_clock(crate::theme::metrics::AUTOSAVE_DEBOUNCE + Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.label_error()),
        Some(shellpp_core::ValidationReason::Required)
    );
    assert_eq!(saved(&ui, cx), "Deploy (prod)");
    // A label typed and then the panel closed at once is saved too.
    ui.type_into(cx, "kc-key-label", "Deploy key");
    ui.click(cx, "panel-close");
    assert_eq!(saved(&ui, cx), "Deploy key");
    // And before an auto-lock (the windows save their edits first).
    ui.click(cx, key_card_id(deploy.record_id));
    ui.type_into(cx, "kc-key-label", " 2");
    ui.state.update(cx, |_, cx| {
        cx.emit(crate::state::AppStateEvent::AutoLockDue)
    });
    cx.run_until_parked();
    assert_eq!(saved(&ui, cx), "Deploy key 2");
    ui.finish(cx);
}

/// The arrows move the selection over the key cards (the panel follows,
/// the shown key hidden again); Esc closes the panel.
#[gpui_kit::test]
fn the_arrows_move_over_the_keys(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, sample);
    let shown = ui.screen.read_with(cx, |screen, _| screen.shown_key_ids());
    assert_eq!(shown.len(), 3);
    let selected =
        |ui: &Ui, cx: &mut TestAppContext| ui.screen.read_with(cx, |screen, _| screen.selected());
    ui.click(cx, key_card_id(shown[0]));
    ui.click(cx, "kc-key-show");
    let first = ui.panel(cx);
    // The click on Show moved the focus; back to the cards.
    ui.click(cx, key_card_id(shown[0]));
    ui.frame(cx, |window, cx| window.press("right", cx));
    assert_eq!(selected(&ui, cx), Some(shown[1]));
    assert!(!first.read_with(cx, |panel, _| panel.is_revealed()));
    assert_eq!(
        ui.panel(cx).read_with(cx, |panel, _| panel.key_id()),
        shown[1]
    );
    ui.frame(cx, |window, cx| window.press("right", cx));
    ui.frame(cx, |window, cx| window.press("right", cx));
    assert_eq!(selected(&ui, cx), Some(shown[2]), "stops at the last key");
    ui.frame(cx, |window, cx| window.press("left", cx));
    assert_eq!(selected(&ui, cx), Some(shown[1]));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert_eq!(selected(&ui, cx), None);
    assert!(!ui.visible(cx, "kc-panel"));
    ui.finish(cx);
}

// -- Generate key (7.1) -----------------------------------------------------------------

#[gpui_kit::test]
fn generate_key_offers_every_type_and_shows_progress(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    ui.click(cx, "kc-new-key-menu");
    assert!(ui.visible(cx, "kc-menu-generate"));
    assert!(ui.visible(cx, "kc-menu-new-identity"));
    ui.click(cx, "kc-menu-generate");
    assert!(ui.visible(cx, "kc-generate"));
    let form = ui.generate_form(cx);
    // Ed25519 by default; ECDSA P-256/384/521; RSA 3072 (default) or 4096.
    assert_eq!(form.read_with(cx, |form, _| form.kind()), KeyKind::Ed25519);
    ui.click(cx, "kc-gen-type-ecdsa");
    assert_eq!(
        form.read_with(cx, |form, _| form.kind()),
        KeyKind::EcdsaP256
    );
    for (id, kind) in [
        ("kc-gen-curve-p384", KeyKind::EcdsaP384),
        ("kc-gen-curve-p521", KeyKind::EcdsaP521),
        ("kc-gen-curve-p256", KeyKind::EcdsaP256),
    ] {
        ui.click(cx, id);
        assert_eq!(form.read_with(cx, |form, _| form.kind()), kind);
    }
    ui.click(cx, "kc-gen-type-rsa");
    assert_eq!(form.read_with(cx, |form, _| form.kind()), KeyKind::Rsa3072);
    assert!(ui.visible(cx, "kc-gen-bits-3072"));
    assert!(ui.visible(cx, "kc-gen-bits-4096"));
    ui.click(cx, "kc-gen-bits-4096");
    assert_eq!(form.read_with(cx, |form, _| form.kind()), KeyKind::Rsa4096);
    ui.click(cx, "kc-gen-type-ed25519");
    assert!(!ui.visible(cx, "kc-gen-bits-3072"));

    // The label is required.
    ui.click(cx, "kc-gen-submit");
    assert_eq!(
        form.read_with(cx, |form, _| form.errors().get("label").copied()),
        Some(shellpp_core::ValidationReason::Required)
    );
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.keys().is_empty())
    );

    // Generate: the progress shows until the key is there.
    ui.type_into(cx, "kc-gen-label", "Work laptop");
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("kc-gen-submit", cx);
        window.render_frame(cx);
        assert!(form.read(cx).is_generating());
        assert!(
            window
                .try_find("kc-gen-progress")
                .is_some_and(|found| found.visible())
        );
    })
    .unwrap();
    cx.run_until_parked();
    let key = ui.key_by_label(cx, "Work laptop");
    assert_eq!(key.algorithm, "ssh-ed25519");
    assert!(!key.encrypted);
    // The new key's panel opens.
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(key.record_id)
    );
    assert!(ui.visible(cx, "kc-key-panel"));
    ui.finish(cx);
}

/// Every type generates a key of its algorithm; a passphrase with "Ask on
/// every connection" encrypts the key without saving it.
#[gpui_kit::test]
fn generated_keys_have_their_type_and_passphrase(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    let run = |ui: &Ui, cx: &mut TestAppContext, label: &str, choose: &[&'static str]| {
        ui.click(cx, "kc-new-key-menu");
        ui.click(cx, "kc-menu-generate");
        for id in choose {
            ui.click(cx, *id);
        }
        ui.type_into(cx, "kc-gen-label", label);
        ui.click(cx, "kc-gen-submit");
        ui.key_by_label(cx, label)
    };
    let p384 = run(
        &ui,
        cx,
        "P-384",
        &["kc-gen-type-ecdsa", "kc-gen-curve-p384"],
    );
    assert_eq!(p384.algorithm, "ecdsa-sha2-nistp384");
    let p521 = run(
        &ui,
        cx,
        "P-521",
        &["kc-gen-type-ecdsa", "kc-gen-curve-p521"],
    );
    assert_eq!(p521.algorithm, "ecdsa-sha2-nistp521");
    let rsa = run(&ui, cx, "RSA", &["kc-gen-type-rsa"]);
    assert_eq!(rsa.algorithm, "ssh-rsa");

    // With a passphrase, asked on every connection.
    ui.click(cx, "kc-new-key-menu");
    ui.click(cx, "kc-menu-generate");
    assert!(!ui.visible(cx, "kc-gen-passphrase-ask"));
    ui.type_into(cx, "kc-gen-passphrase", &new_passphrase());
    assert!(ui.visible(cx, "kc-gen-passphrase-store"));
    ui.click(cx, "kc-gen-passphrase-ask");
    ui.type_into(cx, "kc-gen-label", "Protected");
    ui.click(cx, "kc-gen-submit");
    let protected = ui.key_by_label(cx, "Protected");
    assert!(protected.encrypted);
    assert!(!protected.passphrase_saved);
    ui.finish(cx);
}

// -- New key: the import ----------------------------------------------------------------

#[gpui_kit::test]
fn import_inspects_the_text_and_asks_for_a_passphrase(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    ui.click(cx, "kc-new-key");
    assert!(ui.visible(cx, "kc-import"));
    let form = ui.import_form(cx);
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Empty
    );
    assert!(!form.read_with(cx, |form, cx| form.can_import(cx)));

    // An unencrypted OpenSSH key: the format, no passphrase.
    ui.paste_key(cx, &key_text(KeyKind::Ed25519, "plain@test", None));
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Key {
            format: "OpenSSH",
            encrypted: false
        }
    );
    assert!(ui.visible(cx, "kc-import-format"));
    assert!(!ui.visible(cx, "kc-import-passphrase"));
    // The label comes from the key comment when left empty.
    ui.click(cx, "kc-import-submit");
    let plain = ui.key_by_label(cx, "plain@test");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(plain.record_id)
    );

    // An encrypted key: the passphrase with the save / ask choice.
    let passphrase = new_passphrase();
    ui.click(cx, "kc-new-key");
    let form = ui.import_form(cx);
    ui.paste_key(
        cx,
        &key_text(KeyKind::Ed25519, "locked@test", Some(&passphrase)),
    );
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Key {
            format: "OpenSSH",
            encrypted: true
        }
    );
    for id in [
        "kc-import-passphrase",
        "kc-import-passphrase-store",
        "kc-import-passphrase-ask",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    // A wrong passphrase is refused with its reason.
    ui.type_into(cx, "kc-import-passphrase", "not-the-passphrase");
    ui.click(cx, "kc-import-submit");
    assert_eq!(
        form.read_with(cx, |form, _| form.errors().get("passphrase").copied()),
        Some(shellpp_core::ValidationReason::Invalid)
    );
    assert!(ui.import_form(cx) == form, "the form stays open");
    // The right one, asked on every connection.
    ui.clear(cx, "kc-import-passphrase");
    ui.type_into(cx, "kc-import-passphrase", &passphrase);
    ui.click(cx, "kc-import-passphrase-ask");
    ui.type_into(cx, "kc-import-label", "Locked");
    ui.click(cx, "kc-import-submit");
    let locked = ui.key_by_label(cx, "Locked");
    assert!(locked.encrypted && !locked.passphrase_saved);
    assert!(ui.visible(cx, "kc-key-passphrase-note"));

    // Saved in the vault (the default choice). The comment of an encrypted
    // key is inside its encrypted part: without a label, the type names it.
    let second = new_passphrase();
    ui.click(cx, "kc-new-key");
    ui.paste_key(
        cx,
        &key_text(KeyKind::EcdsaP256, "saved@test", Some(&second)),
    );
    ui.type_into(cx, "kc-import-passphrase", &second);
    ui.click(cx, "kc-import-submit");
    let saved = ui.key_by_label(cx, "ecdsa-sha2-nistp256");
    assert!(saved.encrypted && saved.passphrase_saved);
    ui.finish(cx);
}

/// A text that is not a usable key shows the type "unknown" with the
/// reason, and Import stays off (UI 7.2, D32).
#[gpui_kit::test]
fn import_explains_an_unknown_key(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    ui.click(cx, "kc-new-key");
    let form = ui.import_form(cx);
    // While `inspect` runs (on a background thread), Import waits.
    let text = key_text(KeyKind::Ed25519, "checking@test", None);
    cx.update_window(ui.handle, |_, window, cx| {
        form.update(cx, |form, cx| form.set_text(&text, window, cx));
        assert_eq!(form.read(cx).inspection(), &Inspection::Checking);
        assert!(!form.read(cx).can_import(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(form.read_with(cx, |form, cx| form.can_import(cx)));
    // A public key line instead of a private key.
    let private = key_text(KeyKind::Ed25519, "pub@test", None);
    let parsed = ssh_key::PrivateKey::from_openssh(private.as_bytes()).unwrap();
    let line = parsed.public_key().to_openssh().unwrap();
    ui.paste_key(cx, &line);
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Problem(KeyProblem::NotAKey)
    );
    assert!(ui.visible(cx, "kc-import-unknown"));
    assert!(ui.visible(cx, "kc-import-reason"));
    assert!(!form.read_with(cx, |form, cx| form.can_import(cx)));
    ui.click(cx, "kc-import-submit");
    assert!(ui.keys(cx).is_empty());
    // A damaged key.
    let damaged: String = private.lines().take(3).collect::<Vec<_>>().join("\n");
    ui.paste_key(cx, &damaged);
    assert!(matches!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Problem(_)
    ));
    assert_eq!(
        strings::problem(&KeyProblem::RsaTooSmall),
        "RSA keys under 2048 bits are not accepted."
    );
    // Cancel closes the form.
    ui.click(cx, "kc-import-cancel");
    assert!(!ui.visible(cx, "kc-panel"));
    ui.finish(cx);
}

/// A key that is already in the Keychain (same SHA256 fingerprint) is not
/// created again: its panel opens.
#[gpui_kit::test]
fn a_duplicate_selects_the_existing_key(cx: &mut TestAppContext) {
    let text = key_text(KeyKind::Ed25519, "dup@test", None);
    let seeded = text.clone();
    let ui = Ui::open(cx, move |core, vault| {
        import(core, vault, "Original", &seeded);
    });
    let original = ui.key_by_label(cx, "Original");
    ui.click(cx, "kc-new-key");
    ui.paste_key(cx, &text);
    ui.type_into(cx, "kc-import-label", "Copy");
    ui.click(cx, "kc-import-submit");
    assert_eq!(ui.keys(cx).len(), 1);
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(original.record_id)
    );
    assert_eq!(
        ui.panel(cx).read_with(cx, |panel, _| panel.key_id()),
        original.record_id
    );
    ui.finish(cx);
}

/// Choose file and a file dropped on the form put its text in the form.
#[gpui_kit::test]
fn a_key_file_is_chosen_or_dropped(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    let file = ui.dir.path().join("id_dropped");
    std::fs::write(&file, key_text(KeyKind::Ed25519, "dropped@test", None)).unwrap();
    ui.click(cx, "kc-new-key");
    let form = ui.import_form(cx);
    let bounds = ui.frame(cx, |window, _| window.find("kc-import").bounds());
    let position = bounds.center();
    let dropped = file.clone();
    ui.frame(cx, |window, cx| {
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([dropped].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    });
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Key {
            format: "OpenSSH",
            encrypted: false
        }
    );

    // Choose file: the system dialog (simulated) picks the file.
    let chosen = ui.dir.path().join("id_chosen");
    std::fs::write(&chosen, key_text(KeyKind::EcdsaP256, "chosen@test", None)).unwrap();
    ui.click(cx, "kc-import-file");
    let pick = chosen.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![pick]));
    cx.run_until_parked();
    ui.click(cx, "kc-import-submit");
    assert_eq!(
        ui.key_by_label(cx, "chosen@test").algorithm,
        "ecdsa-sha2-nistp256"
    );

    // A file too large for a key is refused with the reason.
    ui.click(cx, "kc-new-key");
    let big = ui.dir.path().join("big");
    std::fs::write(&big, vec![b'a'; (MAX_KEY_FILE_BYTES + 1) as usize]).unwrap();
    let form = ui.import_form(cx);
    ui.frame(cx, |window, cx| {
        form.update(cx, |form, cx| form.load_file(big, window, cx))
    });
    assert_eq!(
        form.read_with(cx, |form, _| form.file_problem()),
        Some(FileProblem::TooLarge)
    );
    assert!(ui.visible(cx, "kc-import-file-error"));
    ui.finish(cx);
}

// -- Empty state ------------------------------------------------------------------------

#[gpui_kit::test]
fn the_empty_state_offers_generate_import_and_ssh(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    for id in [
        "kc-empty",
        "kc-empty-generate",
        "kc-empty-import",
        "kc-empty-import-ssh",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    // One group: the three buttons as wide as the widest, and Certificate
    // and FIDO2 in the action bar (design-system 6 [decision]).
    let width = |cx: &mut TestAppContext, id: &'static str| {
        ui.frame(cx, |window, _| window.find(id).bounds().size.width)
    };
    let generate = width(cx, "kc-empty-generate");
    assert_eq!(generate, width(cx, "kc-empty-import"));
    assert_eq!(generate, width(cx, "kc-empty-import-ssh"));
    assert_eq!(width(cx, "kc-certificate"), width(cx, "kc-fido2"));
    ui.click(cx, "kc-empty-generate");
    assert!(ui.visible(cx, "kc-generate"));
    ui.click(cx, "kc-empty-import");
    assert!(ui.visible(cx, "kc-import"));
    assert!(!ui.visible(cx, "kc-import-ssh-list"));

    // Import from ~/.ssh: only the private keys of the (temporary) folder.
    let ssh = ui.dir.path().join("ssh");
    let ed25519 = key_text(KeyKind::Ed25519, "ssh@test", None);
    std::fs::write(ssh.join("id_ed25519"), &ed25519).unwrap();
    let public = ssh_key::PrivateKey::from_openssh(ed25519.as_bytes())
        .unwrap()
        .public_key()
        .to_openssh()
        .unwrap();
    std::fs::write(ssh.join("id_ed25519.pub"), &public).unwrap();
    std::fs::write(ssh.join("config"), "Host *\n  User test\n").unwrap();
    std::fs::write(ssh.join("known_hosts"), format!("example.com {public}\n")).unwrap();
    std::fs::write(
        ssh.join("work.ppk"),
        "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\nComment: test\n",
    )
    .unwrap();
    ui.click(cx, "kc-empty-import-ssh");
    assert!(ui.visible(cx, "kc-import-ssh-list"));
    let form = ui.import_form(cx);
    let names: Vec<String> = match form.read_with(cx, |form, _| form.ssh_scan().cloned()) {
        Some(SshScan::Found(files)) => files.into_iter().map(|file| file.name).collect(),
        other => panic!("not scanned: {other:?}"),
    };
    assert_eq!(names, ["id_ed25519", "work.ppk"]);
    ui.click(cx, ssh_file_id("id_ed25519"));
    assert_eq!(
        form.read_with(cx, |form, _| form.inspection().clone()),
        Inspection::Key {
            format: "OpenSSH",
            encrypted: false
        }
    );
    ui.click(cx, "kc-import-submit");
    assert_eq!(ui.key_by_label(cx, "ssh@test").algorithm, "ssh-ed25519");
    assert!(!ui.visible(cx, "kc-empty"));
    ui.finish(cx);
}

/// No folder (a test build without an override, or one that does not
/// exist): the form says so.
#[gpui_kit::test]
fn import_from_ssh_without_a_folder(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, |_, _| {});
    let missing = ui.dir.path().join("no-such-folder");
    cx.update(|cx| set_ssh_folder(Some(missing), cx));
    ui.click(cx, "kc-empty-import-ssh");
    let form = ui.import_form(cx);
    assert_eq!(
        form.read_with(cx, |form, _| form.ssh_scan().cloned()),
        Some(SshScan::Missing)
    );
    cx.update(|cx| set_ssh_folder(None, cx));
    ui.click(cx, "kc-empty-import-ssh");
    let form = ui.import_form(cx);
    assert_eq!(
        form.read_with(cx, |form, _| form.ssh_scan().cloned()),
        Some(SshScan::Missing)
    );
    ui.finish(cx);
}

/// A test build never offers the real `~/.ssh`, even without an override
/// and without `SHELLPP_TEST=1`.
#[gpui_kit::test]
fn a_test_build_reads_no_real_ssh_folder(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(cx.try_global::<SshFolder>().is_none());
        assert_eq!(default_ssh_folder(cx), None);
    });
}

#[test]
fn the_ssh_scan_skips_what_is_not_a_private_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = key_text(KeyKind::Ed25519, "scan@test", None);
    std::fs::write(dir.path().join("id_b"), &key).unwrap();
    std::fs::write(dir.path().join("id_a"), &key).unwrap();
    std::fs::write(dir.path().join("id_a.pub"), "ssh-ed25519 AAAA x").unwrap();
    std::fs::write(dir.path().join("authorized_keys"), "ssh-ed25519 AAAA x").unwrap();
    std::fs::write(dir.path().join("huge"), vec![b'-'; 70 * 1024]).unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    let found = scan_ssh_folder(dir.path()).unwrap();
    let names: Vec<&str> = found.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["id_a", "id_b"]);
    assert_eq!(scan_ssh_folder(&dir.path().join("missing")), None);
    assert_eq!(
        read_key_file(&dir.path().join("huge")).unwrap_err(),
        FileProblem::TooLarge
    );
    assert_eq!(
        read_key_file(&PathBuf::from("Z:/no/such/file")).unwrap_err(),
        FileProblem::Unreadable
    );
}

// -- The whole app ----------------------------------------------------------------------

/// The app over a temporary profile with the sample keys.
struct App2 {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<crate::app::root::RootView>,
    profile: Option<TestProfile>,
}

impl App2 {
    fn open(cx: &mut TestAppContext, width: f32) -> Self {
        cx.executor().allow_parking();
        install(cx);
        cx.update(|cx| {
            crate::app::root::bind_keys(cx);
            crate::terminal::init(cx);
            set_ssh_folder(None, cx);
        });
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let vault_id = opened.core.vaults().unwrap()[0].vault_id;
        sample(&opened.core, vault_id);
        let state = cx.new(|cx| AppState::new(opened, cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(800.)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| crate::app::open_window_with(state.clone(), options, cx))
            .unwrap();
        let app = Self {
            state,
            handle,
            root,
            profile: Some(profile),
        };
        app.frame(cx);
        app
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        self.frame(cx);
    }

    fn bounds(
        &self,
        cx: &mut TestAppContext,
        id: impl Into<ElementId>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .filter(|found| found.visible())
                .map(|found| found.bounds())
        })
        .unwrap()
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        self.bounds(cx, id).is_some()
    }

    fn screen(&self, cx: &mut TestAppContext) -> Entity<KeychainScreen> {
        cx.update(|cx| {
            let shell = self.root.read(cx).shell().cloned().expect("the shell");
            shell.read(cx).keychain_screen().clone()
        })
    }

    fn deploy(&self, cx: &mut TestAppContext) -> KeyView {
        self.state.read_with(cx, |state, _| {
            state
                .keys()
                .into_iter()
                .find(|key| key.label == "Deploy")
                .unwrap()
        })
    }

    fn finish(mut self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
        drop(self.profile.take());
    }
}

/// The sidebar's Keychain section shows this screen; switching to another
/// section hides a shown private key (UI 7.3: leaving the panel).
#[gpui_kit::test]
fn the_section_shows_the_screen_and_leaving_it_hides_the_key(cx: &mut TestAppContext) {
    let app = App2::open(cx, 1280.);
    assert!(!app.visible(cx, "keychain-screen"));
    app.click(cx, "nav-keychain");
    assert!(app.visible(cx, "keychain-screen"));
    assert!(!app.visible(cx, "section-placeholder"));
    let deploy = app.deploy(cx);
    app.click(cx, key_card_id(deploy.record_id));
    // With the panel open the search wraps below the buttons instead of
    // sliding under FIDO2.
    let fido2 = app.bounds(cx, "kc-fido2").unwrap();
    let search = app.bounds(cx, "kc-search-field").unwrap();
    assert!(
        fido2.right() <= search.left() || fido2.bottom() <= search.top(),
        "{fido2:?} {search:?}"
    );
    app.click(cx, "kc-key-show");
    assert!(app.visible(cx, "kc-key-private"));
    let screen = app.screen(cx);
    let panel = screen
        .read_with(cx, |screen, _| screen.key_panel().cloned())
        .unwrap();
    // Hosts, then back: the panel is still open, the key hidden again.
    app.click(cx, "nav-hosts");
    assert!(!panel.read_with(cx, |panel, _| panel.is_revealed()));
    app.click(cx, "nav-keychain");
    assert!(app.visible(cx, "kc-key-panel"));
    assert!(app.visible(cx, "kc-key-hidden-note"));
    assert!(!app.visible(cx, "kc-key-private"));
    app.finish(cx);
}

/// The minimum window (960 px): the open panel collapses the sidebar into
/// its strip of icons, and the cards keep at least 360 px.
#[gpui_kit::test]
fn the_panel_collapses_the_sidebar_in_a_narrow_window(cx: &mut TestAppContext) {
    let app = App2::open(cx, 960.);
    app.click(cx, "nav-keychain");
    assert_eq!(
        app.bounds(cx, "sidebar").unwrap().size.width,
        metrics::WIDTH_SIDEBAR
    );
    let deploy = app.deploy(cx);
    app.click(cx, key_card_id(deploy.record_id));
    app.frame(cx);
    assert_eq!(
        app.bounds(cx, "sidebar").unwrap().size.width,
        crate::shell::sidebar::WIDTH_COLLAPSED
    );
    assert_eq!(
        app.bounds(cx, "kc-panel").unwrap().size.width,
        metrics::WIDTH_DETAILS
    );
    let main = app.bounds(cx, "kc-main").unwrap();
    assert!(
        main.size.width >= crate::shell::details::MAIN_AREA_MIN,
        "{main:?}"
    );
    // The action bar's buttons stay inside the main area, and the search
    // never slides under FIDO2: it wraps below when there is no room.
    for id in ["kc-new-key", "kc-fido2", "kc-sort"] {
        let button = app.bounds(cx, id).unwrap();
        assert!(button.right() <= main.right() + px(0.5), "{id} {button:?}");
    }
    let fido2 = app.bounds(cx, "kc-fido2").unwrap();
    let search = app.bounds(cx, "kc-search-field").unwrap();
    assert!(
        fido2.right() <= search.left() || fido2.bottom() <= search.top(),
        "{fido2:?} {search:?}"
    );
    app.click(cx, "panel-close");
    assert_eq!(
        app.bounds(cx, "sidebar").unwrap().size.width,
        metrics::WIDTH_SIDEBAR
    );
    app.finish(cx);
}

#[test]
fn pure_texts() {
    assert_eq!(key_type("ssh-ed25519"), Some("ED25519"));
    assert_eq!(key_type("ecdsa-sha2-nistp384"), Some("ECDSA P-384"));
    assert_eq!(key_type("ecdsa-sha2-nistp521"), Some("ECDSA P-521"));
    assert_eq!(key_type("unknown"), None);
    assert_eq!(
        strings::format_text("PuTTY (.ppk)", true),
        "PuTTY (.ppk), protected by a passphrase"
    );
    assert_eq!(
        panel::check_label("  "),
        Err(shellpp_core::ValidationReason::Required)
    );
    assert_eq!(
        panel::check_label(&"x".repeat(257)),
        Err(shellpp_core::ValidationReason::TooLong)
    );
    assert_eq!(panel::check_label(" ok "), Ok(()));
    // Every refusal reason has its own sentence.
    for problem in [
        KeyProblem::NotAKey,
        KeyProblem::Damaged,
        KeyProblem::Algorithm("ssh-dss".into()),
        KeyProblem::RsaTooSmall,
        KeyProblem::Cipher("DES-EDE3-CBC".into()),
        KeyProblem::KdfTooExpensive,
        KeyProblem::PassphraseNeeded,
        KeyProblem::WrongPassphrase,
    ] {
        let text = strings::problem(&problem);
        assert!(text.ends_with('.'), "{text}");
    }
}
