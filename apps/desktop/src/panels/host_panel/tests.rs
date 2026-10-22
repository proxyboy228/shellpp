//! Headless tests of the host panel's credentials, the "Show more" fields,
//! the Telnet block, the minimum window size and persistence across a
//! restart (owner's stage 1, step 1.3 and 1.4; lead decisions 2 and 4).
//! GPUI `TestAppContext`, no real window, a temporary profile and the
//! in-memory secret store (never the OS keychain). SSH keys and
//! certificates are generated at run time.

use std::convert::Infallible;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};
use shellpp_core::ValidationReason;
use ssh_key::certificate::{Builder, CertType};
use ssh_key::rand_core::{TryCryptoRng, TryRng};
use ssh_key::{Algorithm, LineEnding, PrivateKey};
use uuid::Uuid;

use super::{CredentialKind, HostPanel};
use crate::panels::menus::HostPanelAction;
use crate::state::AppState;
use crate::testing::TestProfile;
use crate::theme::metrics;

/// A bit more than the autosave debounce.
const AFTER_DEBOUNCE: Duration = Duration::from_millis(600);

/// The OS random generator as a `rand_core` RNG for `ssh-key`.
struct OsRng;

impl TryRng for OsRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(getrandom::u32().expect("os random"))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(getrandom::u64().expect("os random"))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dst).expect("os random");
        Ok(())
    }
}

impl TryCryptoRng for OsRng {}

fn new_key(comment: &str) -> PrivateKey {
    let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("ed25519");
    key.set_comment(comment);
    key
}

fn openssh(key: &PrivateKey, passphrase: Option<&str>) -> String {
    let key = match passphrase {
        Some(passphrase) => key.encrypt(&mut OsRng, passphrase).expect("encrypt"),
        None => key.clone(),
    };
    key.to_openssh(LineEnding::LF).expect("openssh").to_string()
}

/// A user certificate for `subject`, signed by a fresh CA.
fn user_certificate(subject: &PrivateKey) -> String {
    let ca = new_key("ca");
    let mut builder = Builder::new(
        vec![9u8; 32],
        subject.public_key().key_data().clone(),
        0,
        u64::from(u32::MAX),
    )
    .expect("builder");
    builder.cert_type(CertType::User).expect("type");
    builder.key_id("panel-test").expect("key id");
    builder.valid_principal("ops").expect("principal");
    builder
        .sign(&ca)
        .expect("signed")
        .to_openssh()
        .expect("openssh")
}

/// A passphrase generated at run time.
fn passphrase() -> String {
    let bytes: [u8; 8] = std::array::from_fn(|_| getrandom::u32().expect("random") as u8);
    format!(
        "pp-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn install(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::testing::init_ui(cx);
    });
}

fn setup(cx: &mut TestAppContext) -> Entity<AppState> {
    install(cx);
    cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx))
}

fn options(width: gpui::Pixels, height: gpui::Pixels) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(width, height),
        ))),
        ..Default::default()
    }
}

fn open_panel_sized(
    cx: &mut TestAppContext,
    state: &Entity<AppState>,
    record_id: Option<Uuid>,
    width: gpui::Pixels,
    height: gpui::Pixels,
) -> (AnyWindowHandle, Entity<HostPanel>) {
    let state = state.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options(width, height), cx, |window, cx| {
            cx.new(|cx| HostPanel::new(state, record_id, window, cx))
        })
    })
    .unwrap()
}

/// As wide as the details panel and tall enough to show every block.
fn open_panel(
    cx: &mut TestAppContext,
    state: &Entity<AppState>,
    record_id: Option<Uuid>,
) -> (AnyWindowHandle, Entity<HostPanel>) {
    open_panel_sized(
        cx,
        state,
        record_id,
        metrics::at_100(metrics::WIDTH_DETAILS),
        px(4000.),
    )
}

fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(AFTER_DEBOUNCE);
    cx.run_until_parked();
}

fn new_host(cx: &mut TestAppContext, state: &Entity<AppState>, address: &str) -> Uuid {
    state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({ "address": address })), cx)
            .unwrap()
            .record_id
    })
}

fn stored(cx: &mut TestAppContext, state: &Entity<AppState>, id: Uuid) -> Map<String, Value> {
    cx.update(|cx| state.read(cx).host(id).expect("host").data_map())
}

/// Opens "+" and picks `kind` (UI 3.1).
fn open_credential(cx: &mut TestAppContext, handle: AnyWindowHandle, kind: CredentialKind) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-add-credential", cx);
        window.render_frame(cx);
        window.click(format!("host-add-credential-{}", kind.as_str()), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A move made elsewhere (a drag onto a group card, Move to) while the panel
/// still has an edit waiting for the autosave: the autosave keeps the new
/// group instead of moving the host back.
#[gpui_kit::test]
fn a_pending_autosave_keeps_a_move_made_elsewhere(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.27");
    let group = state.update(cx, |state, cx| {
        state
            .save_group(None, map(json!({"name": "Lab"})), cx)
            .unwrap()
            .record_id
    });
    let (handle, panel) = open_panel(cx, &state, Some(host));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.input("db one", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(panel.read(cx).has_pending_save()));
    state.update(cx, |state, cx| {
        state.move_records(vec![host], Some(group), cx).unwrap()
    });
    settle(cx);
    let data = stored(cx, &state, host);
    assert_eq!(data["label"], json!("db one"), "the edit is saved");
    cx.update(|cx| {
        assert_eq!(state.read(cx).host(host).unwrap().group_id, Some(group));
        assert_eq!(panel.read(cx).draft().group_id, Some(group));
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn show_more_fields_are_saved(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.20");
    let (handle, panel) = open_panel(cx, &state, Some(host));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        // Charset: a select, UTF-8 (default) first.
        window.click("host-charset", cx);
        window.render_frame(cx);
        assert!(window.find("host-charset-utf-8").visible());
        window.click("host-charset-iso-8859-15", cx);
        window.render_frame(cx);
        // Mosh: only the value is saved (lead decision 2).
        window.click("host-mosh", cx);
        // Terminal theme: Graphite (default) or Paper.
        window.click("host-terminal-theme", cx);
        window.render_frame(cx);
        assert!(window.find("host-terminal-theme-graphite").visible());
        window.click("host-terminal-theme-paper", cx);
        window.render_frame(cx);
        // Startup snippet: no snippets exist yet; "None" stays.
        window.click("host-startup-snippet", cx);
        window.render_frame(cx);
        window.click("host-startup-snippet-none", cx);
        window.render_frame(cx);
        window.click("host-output-on", cx);
        window.click("host-keepalive", cx);
        window.input("4000", cx);
    })
    .unwrap();
    settle(cx);
    let data = stored(cx, &state, host);
    assert_eq!(data["charset"], json!("iso-8859-15"));
    assert_eq!(data["ssh"]["mosh"], json!(true));
    assert_eq!(data["terminal_theme"], json!({"builtin": "paper"}));
    assert_eq!(data["record_output"], json!("on"));
    assert!(data["ssh"].get("startup_snippet_id").is_none());
    // Keepalive above 3600 s is refused and not saved.
    assert!(data["ssh"].get("keepalive_s").is_none());
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).errors().get("ssh.keepalive_s"),
            Some(&ValidationReason::OutOfRange)
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("backspace", cx);
        // Back to UTF-8: the default is stored as no value.
        window.click("host-charset", cx);
        window.render_frame(cx);
        window.click("host-charset-utf-8", cx);
    })
    .unwrap();
    settle(cx);
    let data = stored(cx, &state, host);
    assert_eq!(data["ssh"]["keepalive_s"], json!(400));
    assert!(data.get("charset").is_none());

    // A new panel on the same host shows the stored values.
    let (_, again) = open_panel(cx, &state, Some(host));
    cx.update(|cx| {
        let draft = again.read(cx).draft();
        assert!(draft.mosh);
        assert_eq!(draft.terminal_theme.as_deref(), Some("paper"));
        assert_eq!(draft.keepalive, "400");
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn telnet_block_is_added_saved_and_removed(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.21");
    let (handle, panel) = open_panel(cx, &state, Some(host));
    cx.update(|cx| {
        assert!(
            panel
                .read(cx)
                .menu_action_enabled(HostPanelAction::AddTelnet)
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("host-telnet-port").is_none());
        window.click("host-add-telnet", cx);
        window.render_frame(cx);
        // The focus goes to the Telnet port; 23 is the placeholder.
        assert_eq!(window.find("host-telnet-port").focused(), Some(true));
        assert!(window.try_find("host-add-telnet").is_none());
        window.input("0", cx);
        window.click("host-telnet-username", cx);
        window.input("admin", cx);
    })
    .unwrap();
    settle(cx);
    // An invalid port is flagged and not saved; the username is.
    assert_eq!(
        stored(cx, &state, host)["telnet"],
        json!({"username": "admin"})
    );
    cx.update(|cx| {
        let panel = panel.read(cx);
        assert_eq!(
            panel.errors().get("telnet.port"),
            Some(&ValidationReason::OutOfRange)
        );
        // Telnet exists: the menu's Add Telnet is off.
        assert!(!panel.menu_action_enabled(HostPanelAction::AddTelnet));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-telnet-port", cx);
        window.press("backspace", cx);
        window.input("2323", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        stored(cx, &state, host)["telnet"],
        json!({"port": 2323, "username": "admin"})
    );
    // The block is removable; the member goes away.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-telnet-remove", cx);
        window.render_frame(cx);
        assert!(window.find("host-add-telnet").visible());
    })
    .unwrap();
    settle(cx);
    assert!(stored(cx, &state, host).get("telnet").is_none());

    // The "…" menu's Add Telnet works on a new host too: the block is saved
    // with the host once its address is valid (the draft rule).
    let (handle, draft) = open_panel(cx, &state, None);
    cx.update_window(handle, |_, window, cx| {
        draft.update(cx, |panel, cx| {
            panel.run_menu_action(HostPanelAction::AddTelnet, window, cx)
        });
        window.render_frame(cx);
        window.click("host-telnet-username", cx);
        window.input("root", cx);
        window.click("host-address", cx);
        window.input("192.0.2.22", cx);
    })
    .unwrap();
    settle(cx);
    let id = cx.update(|cx| draft.read(cx).record_id().expect("created"));
    assert_eq!(
        stored(cx, &state, id)["telnet"],
        json!({"username": "root"})
    );
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn identity_is_created_inline_linked_and_unlinked(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.23");
    let (handle, panel) = open_panel(cx, &state, Some(host));
    open_credential(cx, handle, CredentialKind::Identity);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // No identity exists yet: the creation fields show at once.
        assert_eq!(window.find("host-identity-label").focused(), Some(true));
        // Create without a label: refused with the reason.
        window.click("host-identity-create", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).credential_errors().get("label"),
            Some(&ValidationReason::Required)
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("host-identity-label", cx);
        window.input("Ops login", cx);
        window.click("host-identity-username", cx);
        window.input("ops", cx);
        window.click("host-identity-password", cx);
        window.input("generated-pw-1", cx);
        window.click("host-identity-create", cx);
        window.render_frame(cx);
        // Linked: a row, and Identity left the "+" list.
        assert!(window.find("host-cred-identity").visible());
        assert!(window.try_find("host-identity-form").is_none());
    })
    .unwrap();
    settle(cx);
    let identity = cx.update(|cx| {
        let identities = state.read(cx).identities();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].label, "Ops login");
        assert_eq!(identities[0].username.as_deref(), Some("ops"));
        assert!(identities[0].password_set);
        assert!(
            !panel
                .read(cx)
                .available_credentials()
                .contains(&CredentialKind::Identity)
        );
        identities[0].record_id
    });
    assert_eq!(
        stored(cx, &state, host)["ssh"]["identity_id"],
        json!(identity)
    );

    // Another host picks the saved identity from the list.
    let other = new_host(cx, &state, "192.0.2.24");
    let (handle2, panel2) = open_panel(cx, &state, Some(other));
    open_credential(cx, handle2, CredentialKind::Identity);
    cx.update_window(handle2, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-identity-new").visible());
        window.click(format!("host-identity-{identity}"), cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        stored(cx, &state, other)["ssh"]["identity_id"],
        json!(identity)
    );
    cx.update(|cx| assert_eq!(panel2.read(cx).draft().identity_id, Some(identity)));

    // The row's remove button unlinks it; the identity record stays.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-cred-remove-identity", cx);
    })
    .unwrap();
    settle(cx);
    assert!(stored(cx, &state, host)["ssh"].get("identity_id").is_none());
    cx.update(|cx| assert_eq!(state.read(cx).identities().len(), 1));
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn keys_are_imported_with_the_passphrase_choice_and_linked(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.25");
    let (handle, panel) = open_panel(cx, &state, Some(host));

    // Not a key: refused with the reason, nothing linked.
    open_credential(cx, handle, CredentialKind::Key);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        panel.update(cx, |panel, cx| panel.set_key_text("not a key", window, cx));
        window.render_frame(cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).credential_errors().get("private_key"),
            Some(&ValidationReason::Invalid)
        );
        assert_eq!(panel.read(cx).draft().key_id, None);
    });

    // An encrypted key asks for its passphrase; "ask on each connection"
    // checks it but does not store it.
    let secret = passphrase();
    let encrypted = new_key("deploy");
    let text = openssh(&encrypted, Some(&secret));
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| panel.set_key_text(&text, window, cx));
        window.render_frame(cx);
        // The comment of an encrypted key is encrypted too: name it here.
        window.click("host-key-label", cx);
        window.input("Deploy", cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(panel.read(cx).key_needs_passphrase());
        assert!(panel.read(cx).credential_errors().is_empty());
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("host-key-passphrase").focused(), Some(true));
        window.input("wrong-passphrase", cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).credential_errors().get("passphrase"),
            Some(&ValidationReason::Invalid)
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-key-passphrase", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(&secret, cx);
        window.click("host-key-passphrase-ask", cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    settle(cx);
    let key = cx.update(|cx| {
        let keys = state.read(cx).keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "Deploy");
        assert!(keys[0].encrypted);
        assert!(!keys[0].passphrase_saved);
        assert_eq!(panel.read(cx).credential_form(), None);
        keys[0].record_id
    });
    assert_eq!(stored(cx, &state, host)["ssh"]["key_id"], json!(key));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-cred-key").visible());
        window.click("host-cred-remove-key", cx);
    })
    .unwrap();
    settle(cx);
    assert!(stored(cx, &state, host)["ssh"].get("key_id").is_none());

    // "Store in the vault" saves the passphrase; a plain key needs none and
    // can be picked from the saved keys later.
    let second = new_key("backup");
    let second_secret = passphrase();
    let text = openssh(&second, Some(&second_secret));
    open_credential(cx, handle, CredentialKind::Key);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("host-key-{key}")).visible());
        panel.update(cx, |panel, cx| panel.set_key_text(&text, window, cx));
        window.render_frame(cx);
        window.click("host-key-label", cx);
        window.input("Backup key", cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-key-passphrase", cx);
        window.input(second_secret.as_str(), cx);
        window.click("host-key-passphrase-store", cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        let keys = state.read(cx).keys();
        let backup = keys
            .iter()
            .find(|view| view.label == "Backup key")
            .expect("imported");
        assert!(backup.passphrase_saved);
        assert_eq!(panel.read(cx).draft().key_id, Some(backup.record_id));
    });
    state.update(cx, |state, _| state.shutdown());
}

#[gpui_kit::test]
fn certificate_attaches_to_the_linked_key_and_fido2_is_not_offered(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.26");
    let (handle, panel) = open_panel(cx, &state, Some(host));

    // Without a key the form explains that a certificate needs one.
    open_credential(cx, handle, CredentialKind::Certificate);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-cert-form").visible());
        assert!(window.try_find("host-cert-text").is_none());
        window.click("host-credential-cancel", cx);
    })
    .unwrap();

    let subject = new_key("subject");
    let text = openssh(&subject, None);
    open_credential(cx, handle, CredentialKind::Key);
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| panel.set_key_text(&text, window, cx));
        window.render_frame(cx);
        window.click("host-key-import", cx);
    })
    .unwrap();
    settle(cx);
    let key = cx.update(|cx| panel.read(cx).draft().key_id.expect("key linked"));

    // A certificate of another key is refused.
    let foreign = user_certificate(&new_key("other"));
    open_credential(cx, handle, CredentialKind::Certificate);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-cert-text", cx);
        window.input(&foreign, cx);
        window.click("host-cert-attach", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            panel.read(cx).credential_errors().get("certificate"),
            Some(&ValidationReason::NotAllowed)
        );
        assert!(!state.read(cx).key_has_certificate(key));
    });
    let line = user_certificate(&subject);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-cert-text", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(&line, cx);
        window.click("host-cert-attach", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        stored(cx, &state, host)["ssh"]["certificate_key_id"],
        json!(key)
    );
    cx.update(|cx| {
        assert!(state.read(cx).key_has_certificate(key));
        // Only Identity is left in "+": a FIDO2 key is not offered before
        // FIDO2 support (UI 3.1, D5).
        assert_eq!(
            panel.read(cx).available_credentials(),
            vec![CredentialKind::Identity]
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-cred-certificate").visible());
        window.click("host-add-credential", cx);
        window.render_frame(cx);
        assert!(window.find("host-add-credential-identity").visible());
        assert!(window.try_find("host-add-credential-fido2").is_none());
    })
    .unwrap();
    state.update(cx, |state, _| state.shutdown());
}

/// Ids whose horizontal extent must stay inside the panel.
const FITTING_IDS: &[&str] = &[
    "host-vault",
    "host-address",
    "host-label",
    "host-parent-group",
    "host-tags",
    "host-color",
    "host-backspace-default",
    "host-backspace-ctrl_h",
    "host-notes",
    "host-share",
    "host-port",
    "host-username",
    "host-password",
    "host-add-credential",
    "host-key-form",
    "host-key-text",
    "host-key-label",
    "host-key-import",
    "host-credential-cancel",
    "host-show-more",
    "host-agent-forwarding",
    "host-startup-snippet",
    "host-jump-add",
    "host-proxy-none",
    "host-proxy-socks5",
    "host-proxy-address",
    "host-proxy-port",
    "host-proxy-username",
    "host-proxy-password",
    "host-env-add",
    "host-env-name-0",
    "host-env-value-0",
    "host-env-remove-0",
    "host-charset",
    "host-keepalive",
    "host-history-default",
    "host-history-off",
    "host-output-default",
    "host-output-off",
    "host-mosh",
    "host-terminal-theme",
    "host-telnet-port",
    "host-telnet-remove",
    "host-telnet-username",
    "host-connect",
];

/// Lead decision 4: at the minimum window (960x600) the details panel is
/// 380 px wide (320 px at its narrowest). Every control of the host panel,
/// with "Show more", Telnet, a proxy, a variable and a "+" form open, fits
/// the panel's width; at 600 px the body scrolls and Connect stays visible
/// at the bottom.
#[gpui_kit::test]
fn the_panel_fits_the_minimum_window(cx: &mut TestAppContext) {
    let state = setup(cx);
    let mut host = None;
    for (index, width) in [
        metrics::at_100(metrics::WIDTH_DETAILS),
        metrics::at_100(metrics::WIDTH_DETAILS_MIN),
    ]
    .into_iter()
    .enumerate()
    {
        let id = new_host(cx, &state, &format!("192.0.2.{}", 27 + index));
        host.get_or_insert(id);
        let host = Some(id);
        let (handle, panel) = open_panel_sized(cx, &state, host, width, px(4000.));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("host-show-more", cx);
            window.render_frame(cx);
            window.click("host-proxy-socks5", cx);
            window.click("host-env-add", cx);
            window.click("host-add-telnet", cx);
            panel.update(cx, |panel, cx| {
                panel.open_credential(CredentialKind::Key, window, cx)
            });
            window.render_frame(cx);
            for id in FITTING_IDS {
                let bounds = window.find(*id).bounds();
                assert!(bounds.origin.x >= px(0.), "{id} starts left of the panel");
                assert!(
                    bounds.origin.x + bounds.size.width <= width,
                    "{id} ends at {:?}, past the {width:?} panel",
                    bounds.origin.x + bounds.size.width
                );
            }
        })
        .unwrap();
        settle(cx);
    }
    // 600 px window minus the 44 px top bar.
    let height = px(600.) - metrics::at_100(metrics::HEIGHT_TOP_BAR);
    let (handle, _) = open_panel_sized(
        cx,
        &state,
        host,
        metrics::at_100(metrics::WIDTH_DETAILS),
        height,
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let connect = window.find("host-connect");
        assert!(connect.visible());
        assert!(connect.bounds().origin.y + connect.bounds().size.height <= height);
        // The blocks below the fold are reached by scrolling the body.
        let body = window.find("host-panel-body").bounds();
        assert!(body.origin.y + body.size.height <= connect.bounds().origin.y);
        assert!(
            window.find("host-block-telnet").bounds().origin.y > body.origin.y + body.size.height
        );
    })
    .unwrap();
    state.update(cx, |state, _| state.shutdown());
}

/// Everything the panel saves survives closing and reopening the profile.
#[gpui_kit::test]
fn panel_values_survive_a_restart(cx: &mut TestAppContext) {
    install(cx);
    let mut profile = TestProfile::new();
    let state = cx.new(|cx| AppState::new(profile.open_unlocked(), cx));
    let host = new_host(cx, &state, "192.0.2.28");
    let (handle, _panel) = open_panel(cx, &state, Some(host));
    open_credential(cx, handle, CredentialKind::Identity);
    cx.update_window(handle, |_, window, cx| {
        window.click("host-identity-label", cx);
        window.input("Saved login", cx);
        window.click("host-identity-password", cx);
        window.input("generated-pw-2", cx);
        window.click("host-identity-create", cx);
        window.render_frame(cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        window.click("host-mosh", cx);
        window.click("host-agent-forwarding", cx);
        window.click("host-proxy-http", cx);
        window.render_frame(cx);
        window.click("host-proxy-address", cx);
        window.input("proxy.example.com", cx);
        window.click("host-proxy-port", cx);
        window.input("3128", cx);
        window.click("host-proxy-username", cx);
        window.input("proxyuser", cx);
        window.click("host-proxy-password", cx);
        window.input("generated-pw-3", cx);
        window.click("host-add-telnet", cx);
        window.render_frame(cx);
        window.input("2323", cx);
    })
    .unwrap();
    settle(cx);
    state.update(cx, |state, _| state.shutdown());
    cx.update_window(handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    let reopened = cx.new(|cx| AppState::new(profile.open_unlocked(), cx));
    cx.update(|cx| {
        let state = reopened.read(cx);
        let host = state.host(host).expect("host after restart");
        let data = host.data_map();
        assert_eq!(data["ssh"]["mosh"], json!(true));
        assert_eq!(data["ssh"]["agent_forwarding"], json!(true));
        assert_eq!(
            data["ssh"]["proxy"],
            json!({"type": "http", "address": "proxy.example.com", "port": 3128, "username": "proxyuser"})
        );
        assert_eq!(data["telnet"], json!({"port": 2323}));
        assert!(
            host.secrets_set
                .iter()
                .any(|path| path == "ssh.proxy.password")
        );
        let identities = state.identities();
        assert_eq!(identities.len(), 1);
        assert_eq!(data["ssh"]["identity_id"], json!(identities[0].record_id));
        assert!(identities[0].password_set);
        let proxy_password = state
            .reveal_secret(host.record_id, "ssh.proxy.password")
            .unwrap();
        assert_eq!(proxy_password.expose_secret(), "generated-pw-3");
    });
    reopened.update(cx, |state, _| state.shutdown());
}

/// Stage 5 (D60): Agent Forwarding on shows its warning and two options;
/// both are saved in `host.ssh`.
#[gpui_kit::test]
fn agent_forwarding_shows_its_warning_and_options(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.21");
    let (handle, _panel) = open_panel(cx, &state, Some(host));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        assert!(window.try_find("host-agent-warning").is_none());
        window.click("host-agent-forwarding", cx);
        window.render_frame(cx);
        assert!(window.find("host-agent-warning").visible());
        window.click("host-agent-system", cx);
        window.render_frame(cx);
        window.click("host-agent-confirm", cx);
    })
    .unwrap();
    settle(cx);
    let data = stored(cx, &state, host);
    assert_eq!(data["ssh"]["agent_forwarding"], json!(true));
    assert_eq!(data["ssh"]["agent_forward_system"], json!(true));
    assert_eq!(data["ssh"]["agent_confirm"], json!(true));
}

/// D80: "Allow legacy algorithms" is off by default; on, its warning stays
/// shown and the host stores it.
#[gpui_kit::test]
fn legacy_algorithms_show_their_warning(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.80");
    let (handle, _panel) = open_panel(cx, &state, Some(host));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        assert!(window.try_find("host-legacy-warning").is_none());
        window.click("host-legacy-algorithms", cx);
        window.render_frame(cx);
        assert!(window.find("host-legacy-warning").visible());
    })
    .unwrap();
    settle(cx);
    let data = stored(cx, &state, host);
    assert_eq!(data["ssh"]["allow_legacy_algorithms"], json!(true));
}

/// Plan of run 4, item 7.4: Esc closes the open dropdown, then the panel
/// (a click on a dropdown leaves the keyboard focus in the field).
#[gpui_kit::test]
fn escape_closes_the_dropdown_then_the_panel(cx: &mut TestAppContext) {
    let state = setup(cx);
    let host = new_host(cx, &state, "192.0.2.74");
    let (handle, panel) = open_panel(cx, &state, Some(host));
    let closed = std::rc::Rc::new(std::cell::Cell::new(false));
    let _subscription = cx.update(|cx| {
        let closed = closed.clone();
        cx.subscribe(&panel, move |_, event: &crate::panels::PanelEvent, _| {
            if matches!(event, crate::panels::PanelEvent::Close) {
                closed.set(true);
            }
        })
    });
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("host-charset-utf-8").is_some()
        })
        .unwrap()
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-label", cx);
        window.click("host-show-more", cx);
        window.render_frame(cx);
        window.click("host-charset", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(shown(cx), "the dropdown is open");
    let escape = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    escape(cx);
    assert!(!shown(cx), "Esc closes the dropdown");
    assert!(!closed.get(), "the panel stays");
    escape(cx);
    assert!(closed.get(), "the second Esc closes the panel");
    state.update(cx, |state, _| state.shutdown());
}
