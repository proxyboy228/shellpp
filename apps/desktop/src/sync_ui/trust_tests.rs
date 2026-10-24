//! Headless tests of the certificate dialogs (SP 2.1; UI 12.2, 13.5 step 1;
//! D187): Account & Sync of a bound device whose pinned certificate changed
//! offers its review (before run 9 only Sign out led to the trust dialog),
//! the changed-certificate dialog shows both fingerprints and offers "Use
//! the system's trust" only when the system trusts the new one, Cancel keeps
//! the pin, the wizard's certificate step offers "Add a CA certificate…",
//! and a CA file is shown for confirmation before anything is kept. The
//! certificates are made up (no server is asked: the core tests and
//! `server/tests/tls_trust.rs` check the choices against a server).

use gpui::{AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, px, size};
use gpui::{WindowBounds, WindowOptions};
use gpui_kit::test::TestWindowExt as _;
use shellpp_core::service::{ServerTrust, SyncStatus, TrustChoice};
use shellpp_core::sync::{CertInfo, TlsProblem};

use super::trust::{self, TrustPanel};
use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;

const ORIGIN: &str = "https://sync.example.com";

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
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
            _root: root,
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

    fn open_trust(&self, problem: TlsProblem, cx: &mut TestAppContext) -> Entity<TrustPanel> {
        let state = self.state.clone();
        let panel = cx
            .update_window(self.handle, move |_, window, cx| {
                let done: trust::Trusted = std::rc::Rc::new(|_, _, _| {});
                trust::open_trust(state, ORIGIN.to_owned(), problem, done, window, cx)
            })
            .unwrap();
        self.settle(cx);
        panel
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn info(seed: u8) -> CertInfo {
    let der = vec![0x30, 0x03, 0x02, 0x01, seed];
    CertInfo {
        subject: "CN=sync.example.com".into(),
        issuer: "CN=Home CA".into(),
        not_before: "2026-10-01T00:00:00Z".into(),
        not_after: "2026-12-01T00:00:00Z".into(),
        sha256: shellpp_core::sync::tls::sha256(&der),
        der,
    }
}

fn changed(system_trusts: bool) -> TlsProblem {
    TlsProblem::PinChanged {
        pinned: info(1).sha256,
        new: info(2),
        system_trusts,
    }
}

/// A bound, signed-in device whose cycle stopped at a changed (or refused)
/// certificate: Account & Sync says so and offers Review, beside the trust
/// row; nothing asks to sign out.
#[gpui::test]
fn account_and_sync_offers_the_review_of_a_changed_certificate(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    core.sync_server_trust(ORIGIN, TrustChoice::Pin(info(1).sha256))
        .expect("pinned");
    for code in ["certificate_changed", "certificate_refused"] {
        let status = SyncStatus {
            bound: true,
            signed_in: true,
            server_url: Some(ORIGIN.to_owned()),
            email: Some("demo@example.com".to_owned()),
            last_error: Some(code.to_owned()),
            ..Default::default()
        };
        ui.state
            .update(cx, |state, cx| state.set_sync_status(status, cx));
        let state = ui.state.clone();
        cx.update_window(ui.handle, move |_, window, cx| {
            crate::settings::open(state, window, cx)
        })
        .unwrap();
        ui.settle(cx);
        ui.click("settings-nav-account", cx);
        assert!(ui.visible("sync-certificate-warning", cx), "{code}");
        assert!(ui.visible("sync-certificate-review", cx), "{code}");
        assert!(ui.visible("sync-trust-row", cx), "the trust row");
        assert!(!ui.visible("sync-sign-in-again", cx), "still signed in");
        cx.update_window(ui.handle, |_, window, cx| {
            gpui_kit::component::WindowExt::close_dialog(window, cx)
        })
        .unwrap();
        ui.settle(cx);
    }
    ui.finish(cx);
}

/// The changed-certificate dialog shows the pinned and the new fingerprint
/// side by side and whether the system trusts the new one; "Use the
/// system's trust" is there only when it does.
#[gpui::test]
fn the_changed_certificate_dialog_offers_the_systems_trust_only_when_it_trusts(
    cx: &mut TestAppContext,
) {
    let ui = Ui::open(cx);
    let _ = ui.open_trust(changed(false), cx);
    for id in [
        "sync-pin-changed",
        "sync-fingerprint-pinned",
        "sync-fingerprint",
        "sync-system-distrusts",
        "sync-pin-new",
        "sync-trust-cancel",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert!(!ui.visible("sync-use-system", cx));
    ui.click("sync-trust-cancel", cx);
    assert!(!ui.visible("sync-trust", cx), "Cancel closes it");

    let _ = ui.open_trust(changed(true), cx);
    assert!(ui.visible("sync-system-trusts", cx));
    assert!(ui.visible("sync-use-system", cx));
    ui.finish(cx);
}

/// Cancel keeps the pin: sync stays stopped and nothing is stored.
#[gpui::test]
fn cancel_keeps_the_pin(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    let pin = info(1).sha256;
    core.sync_server_trust(ORIGIN, TrustChoice::Pin(pin))
        .expect("pinned");
    let _ = ui.open_trust(changed(true), cx);
    ui.click("sync-trust-cancel", cx);
    assert_eq!(
        core.sync_server_trust_info(ORIGIN).expect("info"),
        ServerTrust::Pinned { sha256: pin }
    );
    ui.finish(cx);
}

/// The wizard's certificate step of an unknown issuer: the certificate,
/// "Trust this certificate", "Add a CA certificate…", Back and Cancel. A
/// refusal (expired) has no way to trust.
#[gpui::test]
fn the_wizard_offers_a_pin_or_a_ca_and_refuses_an_expired_certificate(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let state = ui.state.clone();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_connect(state, false, window, cx)
        })
        .unwrap();
    dialog.update(cx, |dialog, cx| {
        dialog.show_trust(ORIGIN.to_owned(), TlsProblem::UnknownIssuer(info(3)), cx)
    });
    ui.settle(cx);
    for id in [
        "sync-trust",
        "sync-fingerprint",
        "sync-pin",
        "sync-ca-browse",
        "sync-back",
        "sync-cancel",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    dialog.update(cx, |dialog, cx| {
        dialog.show_trust(ORIGIN.to_owned(), TlsProblem::Expired(info(4)), cx)
    });
    ui.settle(cx);
    assert!(ui.visible("sync-trust", cx));
    assert!(!ui.visible("sync-pin", cx) && !ui.visible("sync-ca-browse", cx));
    ui.finish(cx);
}

/// "Add a CA certificate…": a file that holds no certificate is refused
/// with a reason; a certificate is shown (subject and fingerprint) for
/// confirmation with "Add this CA" and Back; nothing is stored before the
/// server is asked.
#[gpui::test]
fn a_ca_file_is_shown_for_confirmation(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    let panel = ui.open_trust(TlsProblem::UnknownIssuer(info(5)), cx);
    panel.update(cx, |panel, cx| panel.set_ca_file(b"not a certificate", cx));
    ui.settle(cx);
    assert!(ui.visible("sync-error", cx));
    assert!(!ui.visible("sync-ca-confirm", cx));
    // A CA made now, as a DER file (the core's tests read PEM too).
    let key = rcgen::KeyPair::generate().expect("key");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Home CA");
    let der = params.self_signed(&key).expect("ca").der().to_vec();
    panel.update(cx, |panel, cx| panel.set_ca_file(&der, cx));
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.ca.clone().map(|ca| ca.der)),
        Some(der)
    );
    ui.settle(cx);
    for id in [
        "sync-ca-confirm",
        "sync-ca-subject",
        "sync-ca-fingerprint",
        "sync-ca-add",
        "sync-ca-back",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert_eq!(
        core.sync_server_trust_info(ORIGIN).expect("info"),
        ServerTrust::System,
        "nothing is kept before the server answers"
    );
    ui.click("sync-ca-back", cx);
    assert!(!ui.visible("sync-ca-confirm", cx));
    assert!(ui.visible("sync-pin", cx));
    ui.finish(cx);
}

/// A TLS server on 127.0.0.1 that shows a self-signed certificate for
/// `localhost` and answers `GET /v1/server`; returns its port and the
/// certificate's DER.
fn tls_server() -> (u16, Vec<u8>) {
    use std::io::{Read, Write};
    const INFO: &str = r#"{"version":"0.7.0","api_versions":["v1"],"registration":"open","smtp":false,"limits":{"max_envelope_bytes":1,"max_push_items":1,"max_push_body_bytes":1,"default_pull_limit":1,"max_pull_limit":1,"tombstone_retention_days":90},"server_generation":null}"#;
    let key = rcgen::KeyPair::generate().expect("key");
    let params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).expect("params");
    let cert = params.self_signed(&key).expect("cert");
    let der = cert.der().to_vec();
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = std::sync::Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(der.clone())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
                ),
            )
            .expect("tls config"),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let config = config.clone();
            std::thread::spawn(move || {
                let Ok(mut conn) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::Stream::new(&mut conn, &mut stream);
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{INFO}",
                    INFO.len()
                );
                let _ = tls.write_all(answer.as_bytes());
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    (port, der)
}

/// The dialog of an unknown issuer against a real TLS server: "Trust this
/// certificate" pins the certificate the server shows, closes the dialog
/// and runs its follow-up (Account & Sync: sync now; the wizard: the
/// account step).
#[gpui::test]
fn a_kept_pin_closes_the_dialog_and_goes_on(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    core.sync_set_system_roots_for_test(Vec::new());
    let (port, der) = tls_server();
    let origin = format!("https://localhost:{port}");
    let problem = match core.sync_server_check(&origin).expect("check") {
        shellpp_core::service::ServerCheck::Certificate { problem, .. } => problem,
        other => panic!("expected the trust choice, got {other:?}"),
    };
    let went_on = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = went_on.clone();
    let state = ui.state.clone();
    let shown = origin.clone();
    cx.update_window(ui.handle, move |_, window, cx| {
        let done: trust::Trusted = std::rc::Rc::new(move |_, _, _| flag.set(true));
        trust::open_trust(state, shown, problem, done, window, cx)
    })
    .unwrap();
    ui.settle(cx);
    ui.click("sync-pin", cx);
    ui.settle(cx);
    assert!(went_on.get(), "the follow-up ran");
    assert!(!ui.visible("sync-trust", cx), "the dialog closed");
    assert_eq!(
        core.sync_server_trust_info(&origin).expect("info"),
        ServerTrust::Pinned {
            sha256: shellpp_core::sync::tls::sha256(&der)
        }
    );

    // The wizard's step goes on to the account step.
    let other = tls_server();
    let origin = format!("https://localhost:{}", other.0);
    let problem = match core.sync_server_check(&origin).expect("check") {
        shellpp_core::service::ServerCheck::Certificate { problem, .. } => problem,
        other => panic!("expected the trust choice, got {other:?}"),
    };
    let state = ui.state.clone();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_connect(state, false, window, cx)
        })
        .unwrap();
    dialog.update(cx, |dialog, cx| dialog.show_trust(origin, problem, cx));
    ui.settle(cx);
    ui.click("sync-pin", cx);
    ui.settle(cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step.clone()),
        super::ConnectStep::Account
    );
    ui.finish(cx);
}

/// A pin of another certificate than the one the server shows now (it
/// changed after the dialog opened) is not kept: the dialog stays open on
/// what the server shows now, says so, and the follow-up does not run.
#[gpui::test]
fn a_choice_the_server_refuses_keeps_nothing(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let core = ui.core(cx);
    core.sync_set_system_roots_for_test(Vec::new());
    let (port, der) = tls_server();
    let origin = format!("https://localhost:{port}");
    let went_on = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = went_on.clone();
    let state = ui.state.clone();
    let shown = origin.clone();
    let panel = cx
        .update_window(ui.handle, move |_, window, cx| {
            let done: trust::Trusted = std::rc::Rc::new(move |_, _, _| flag.set(true));
            trust::open_trust(
                state,
                shown,
                TlsProblem::UnknownIssuer(info(9)),
                done,
                window,
                cx,
            )
        })
        .unwrap();
    ui.settle(cx);
    ui.click("sync-pin", cx);
    ui.settle(cx);
    assert!(!went_on.get());
    assert!(ui.visible("sync-trust", cx) && ui.visible("sync-error", cx));
    let now = panel.read_with(cx, |panel, _| panel.problem.clone());
    match now {
        TlsProblem::UnknownIssuer(info) => {
            assert_eq!(info.sha256, shellpp_core::sync::tls::sha256(&der));
        }
        other => panic!("expected what the server shows now, got {other:?}"),
    }
    assert_eq!(
        core.sync_server_trust_info(&origin).expect("info"),
        ServerTrust::System,
        "nothing kept"
    );
    ui.finish(cx);
}

/// The weaker KDF confirmation (D189): both settings side by side, Cancel
/// and the danger button; Cancel closes it and the wizard stays on its
/// account step, signed out.
#[gpui::test]
fn the_weaker_kdf_dialog_shows_both_settings_and_cancel_keeps_the_device_out(
    cx: &mut TestAppContext,
) {
    use shellpp_core::crypto::{KdfParams, PlatformClass};
    let ui = Ui::open(cx);
    let state = ui.state.clone();
    let wizard = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_connect(state, false, window, cx)
        })
        .unwrap();
    let offer = shellpp_core::service::WeakerKdf {
        local: KdfParams::preset(PlatformClass::Desktop),
        server: KdfParams::preset(PlatformClass::Mobile),
    };
    let shown = wizard.clone();
    cx.update_window(ui.handle, move |_, window, cx| {
        super::kdf::open_weaker_kdf(shown, offer, window, cx)
    })
    .unwrap();
    ui.settle(cx);
    for id in [
        "sync-weaker-kdf",
        "sync-kdf-local",
        "sync-kdf-server",
        "sync-kdf-cancel",
        "sync-kdf-continue",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    ui.click("sync-kdf-cancel", cx);
    assert!(!ui.visible("sync-weaker-kdf", cx));
    assert!(ui.visible("sync-connect", cx), "the wizard stays");
    assert_eq!(
        super::kdf::describe(&KdfParams::preset(PlatformClass::Desktop)),
        "64 MiB, 3 passes, 4 lanes"
    );
    assert_eq!(
        super::kdf::describe(&KdfParams::preset(PlatformClass::Mobile)),
        "32 MiB, 4 passes, 2 lanes"
    );
    ui.finish(cx);
}

/// The sign-out warning counts what is not on the server (D190).
#[test]
fn the_sign_out_warning_counts_what_is_not_on_the_server() {
    use super::strings::delete_local_unsynced;
    assert_eq!(delete_local_unsynced(0, 0), None);
    assert!(
        delete_local_unsynced(1, 0)
            .unwrap()
            .starts_with("1 change is not on the server yet:")
    );
    assert!(
        delete_local_unsynced(3, 2)
            .unwrap()
            .starts_with("3 changes are not on the server yet and 2 conflicts are kept only here:")
    );
}
