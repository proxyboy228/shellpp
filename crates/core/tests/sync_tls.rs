//! Certificate trust of the sync client against a real TLS server
//! (docs/sync-protocol.md, section 2.1; D93; plan run 7, section 4,
//! "certificate swap"): the system roots, a pinned certificate, a CA added
//! for one origin; a changed pin stops the request before any byte of it
//! is sent; expired and wrong-name certificates are refused even when
//! pinned. Certificates are generated at run time.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use shellpp_core::sync::{HttpApi, HttpError, TlsProblem, TrustMode};

const SERVER_INFO: &str = r#"{"version":"0.7.0","api_versions":["v1"],"registration":"invite","smtp":false,"limits":{"max_envelope_bytes":1,"max_push_items":1,"max_push_body_bytes":1,"default_pull_limit":1,"max_pull_limit":1,"tombstone_retention_days":90},"server_generation":null}"#;

struct Cert {
    der: CertificateDer<'static>,
    key: Vec<u8>,
}

fn days(n: i64) -> time::Duration {
    time::Duration::days(n)
}

fn ca() -> (CertificateDer<'static>, Issuer<'static, KeyPair>) {
    let key = KeyPair::generate().expect("key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "Shell++ test CA");
    let cert = params.self_signed(&key).expect("ca");
    (cert.der().clone(), Issuer::new(params, key))
}

fn leaf(names: &[&str], issuer: Option<&Issuer<'static, KeyPair>>, valid: (i64, i64)) -> Cert {
    let key = KeyPair::generate().expect("key");
    let mut params =
        CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
            .expect("params");
    params.distinguished_name.push(DnType::CommonName, names[0]);
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now + days(valid.0);
    params.not_after = now + days(valid.1);
    let cert = match issuer {
        Some(issuer) => params.signed_by(&key, issuer).expect("signed"),
        None => params.self_signed(&key).expect("self-signed"),
    };
    Cert {
        der: cert.der().clone(),
        key: key.serialize_der(),
    }
}

fn sha256(der: &[u8]) -> [u8; 32] {
    shellpp_core::sync::tls::sha256(der)
}

/// A TLS server on 127.0.0.1 answering `GET /v1/server`; counts the HTTP
/// requests it actually received. [`TlsServer::swap`] changes the
/// certificate it shows (the same origin).
struct TlsServer {
    port: u16,
    requests: Arc<AtomicUsize>,
    config: Arc<Mutex<Arc<rustls::ServerConfig>>>,
}

impl TlsServer {
    fn origin(&self) -> String {
        format!("https://localhost:{}", self.port)
    }

    fn swap(&self, cert: &Cert) {
        *self.config.lock().unwrap() = server_config(cert);
    }
}

fn server_config(cert: &Cert) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der.clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key.clone())),
        )
        .expect("server config");
    Arc::new(config)
}

fn serve(cert: &Cert) -> TlsServer {
    let config = Arc::new(Mutex::new(server_config(cert)));
    let shown = config.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let config = shown.lock().unwrap().clone();
            let counter = counter.clone();
            thread::spawn(move || {
                let Ok(mut conn) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::Stream::new(&mut conn, &mut stream);
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match tls.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                    if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    SERVER_INFO.len(),
                    SERVER_INFO
                );
                let _ = tls.write_all(response.as_bytes());
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    TlsServer {
        port,
        requests,
        config,
    }
}

/// The TLS problem of a failed check, unboxed for matching.
fn problem(result: Result<(), HttpError>) -> Option<TlsProblem> {
    match result {
        Err(HttpError::Tls(problem)) => Some(*problem),
        _ => None,
    }
}

fn check(server: &TlsServer, trust: TrustMode) -> Result<(), HttpError> {
    let api = HttpApi::new(
        &format!("https://localhost:{}", server.port),
        trust,
        "0.7.0",
    )?;
    api.server_info().map(|_| ())
}

#[test]
fn an_unknown_issuer_is_refused_by_the_system_and_accepted_when_pinned() {
    let cert = leaf(&["localhost"], None, (-1, 30));
    let server = serve(&cert);
    match problem(check(&server, TrustMode::System)) {
        Some(TlsProblem::UnknownIssuer(info)) => {
            assert_eq!(
                info.sha256,
                sha256(&cert.der),
                "the dialog shows this certificate"
            );
        }
        other => panic!("expected unknown_issuer, got {other:?}"),
    }
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "nothing was sent"
    );
    check(&server, TrustMode::Pinned(sha256(&cert.der))).expect("pinned works");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[test]
fn a_swapped_certificate_stops_a_pinned_origin_before_the_request() {
    let pinned = leaf(&["localhost"], None, (-1, 30));
    let swapped = leaf(&["localhost"], None, (-1, 30));
    let server = serve(&swapped);
    match problem(check(&server, TrustMode::Pinned(sha256(&pinned.der)))) {
        Some(TlsProblem::PinChanged {
            pinned: old,
            new,
            system_trusts,
        }) => {
            assert_eq!(old, sha256(&pinned.der));
            assert_eq!(new.sha256, sha256(&swapped.der));
            assert!(!system_trusts);
        }
        other => panic!("expected certificate_changed, got {other:?}"),
    }
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "no request reached the server"
    );
    let error = check(&server, TrustMode::Pinned(sha256(&pinned.der))).unwrap_err();
    assert_eq!(error.code(), "certificate_changed");
}

#[test]
fn a_ca_added_for_the_origin_is_trusted() {
    let (ca_der, issuer) = ca();
    let cert = leaf(&["localhost"], Some(&issuer), (-1, 30));
    let server = serve(&cert);
    check(&server, TrustMode::Ca(ca_der.to_vec())).expect("the added CA trusts it");
    // Another CA does not.
    let (other_ca, _) = ca();
    assert!(matches!(
        check(&server, TrustMode::Ca(other_ca.to_vec())),
        Err(HttpError::Tls(_))
    ));
}

#[test]
fn expired_and_wrong_name_certificates_are_refused_even_when_pinned() {
    let expired = leaf(&["localhost"], None, (-30, -1));
    let server = serve(&expired);
    match problem(check(&server, TrustMode::Pinned(sha256(&expired.der)))) {
        Some(TlsProblem::Expired(_)) => {}
        other => panic!("expected expired, got {other:?}"),
    }
    let wrong = leaf(&["sync.example.com"], None, (-1, 30));
    let server = serve(&wrong);
    match problem(check(&server, TrustMode::Pinned(sha256(&wrong.der)))) {
        Some(TlsProblem::WrongName(_)) => {}
        other => panic!("expected wrong name, got {other:?}"),
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

#[test]
fn a_not_yet_valid_certificate_is_refused_even_when_pinned() {
    let future = leaf(&["localhost"], None, (1, 30));
    let server = serve(&future);
    match problem(check(&server, TrustMode::Pinned(sha256(&future.der)))) {
        Some(TlsProblem::Expired(info)) => assert_eq!(info.sha256, sha256(&future.der)),
        other => panic!("expected expired, got {other:?}"),
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

/// A pin one bit off is another certificate: `certificate_changed` before
/// any request, naming the pin and the presented certificate.
#[test]
fn a_pin_one_bit_off_is_a_changed_pin() {
    let cert = leaf(&["localhost"], None, (-1, 30));
    let server = serve(&cert);
    let right = sha256(&cert.der);
    for (i, bit) in [(0, 0x01), (31, 0x80)] {
        let mut pin = right;
        pin[i] ^= bit;
        match problem(check(&server, TrustMode::Pinned(pin))) {
            Some(TlsProblem::PinChanged {
                pinned,
                new,
                system_trusts,
            }) => {
                assert_eq!(pinned, pin);
                assert_eq!(new.sha256, right);
                assert!(!system_trusts);
            }
            other => panic!("expected certificate_changed, got {other:?}"),
        }
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    check(&server, TrustMode::Pinned(right)).expect("the exact pin");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

/// A pin is the SHA-256 of the DER (FIPS 180-2 vector of "abc"), shown as
/// `openssl x509 -fingerprint -sha256` shows it.
#[test]
fn pins_are_sha256_shown_as_openssl_shows_them() {
    let digest = sha256(b"abc");
    assert_eq!(
        shellpp_core::sync::tls::fingerprint_text(&digest),
        "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
    );
}

#[test]
fn plain_http_is_refused_for_any_host_but_loopback() {
    for origin in [
        "http://sync.example.com",
        "http://192.168.1.10:8080",
        "http://10.0.0.1",
    ] {
        match HttpApi::new(origin, TrustMode::System, "0.7.0") {
            Err(HttpError::Origin(_)) => {}
            other => panic!("{origin}: expected an origin error, got {other:?}"),
        }
    }
    for origin in ["http://localhost:1", "http://127.0.0.1:1", "http://[::1]:1"] {
        assert!(
            HttpApi::new(origin, TrustMode::System, "0.7.0").is_ok(),
            "{origin}"
        );
    }
}

// ---------------------------------------------------------------------------
// The trust dialogs' choices through the core (SP 2.1; UI 12.2, 13.5; D187)
// ---------------------------------------------------------------------------

mod common;

use shellpp_core::service::{ServerCheck, ServerTrust, TrustChoice};
use shellpp_core::sync::SystemRoots;

/// A core of its own (nothing local needed for the server step) whose
/// system's trust is an empty root set: the machine's store has no part.
fn core() -> (common::Env, shellpp_core::Core) {
    let env = common::Env::new();
    let core = env.open();
    core.sync_set_system_roots_for_test(Vec::new());
    (env, core)
}

fn checked_problem(check: ServerCheck) -> TlsProblem {
    match check {
        ServerCheck::Certificate { problem, .. } => problem,
        ServerCheck::Ready { .. } => panic!("expected a certificate problem"),
    }
}

/// "Add a CA certificate…" in the refusal dialog: the server's certificate
/// validates against the CA, so the CA is stored for the origin and the
/// next check is ready; Account & Sync names the CA.
#[test]
fn a_ca_added_in_the_refusal_dialog_is_accepted() {
    let (ca_der, issuer) = ca();
    let server = serve(&leaf(&["localhost"], Some(&issuer), (-1, 30)));
    let (_env, core) = core();
    match checked_problem(core.sync_server_check(&server.origin()).expect("check")) {
        TlsProblem::UnknownIssuer(_) => {}
        other => panic!("expected an unknown issuer, got {other:?}"),
    }
    let check = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::Ca(ca_der.to_vec()))
        .expect("verified");
    assert!(matches!(check, ServerCheck::Ready { .. }), "{check:?}");
    assert_eq!(
        core.sync_server_trust_info(&server.origin()).expect("info"),
        ServerTrust::Ca {
            name: "Shell++ test CA".to_owned(),
            sha256: sha256(&ca_der),
        }
    );
    assert!(matches!(
        core.sync_server_check(&server.origin()).expect("check"),
        ServerCheck::Ready { .. }
    ));
}

/// A CA whose chain names another host, or holds an expired certificate,
/// or that did not issue the server's certificate, is not stored: the
/// answer is the problem under that CA, and the origin keeps its trust.
#[test]
fn a_ca_is_not_stored_for_another_name_an_expired_chain_or_another_issuer() {
    let (ca_der, issuer) = ca();
    let (_, other_issuer) = ca();
    let (_env, core) = core();
    type Expected = fn(&TlsProblem) -> bool;
    let cases: [(Cert, Expected); 3] = [
        (leaf(&["sync.example.com"], Some(&issuer), (-1, 30)), |p| {
            matches!(p, TlsProblem::WrongName(_))
        }),
        (leaf(&["localhost"], Some(&issuer), (-30, -1)), |p| {
            matches!(p, TlsProblem::Expired(_))
        }),
        // Another CA of the same name: its signature does not verify.
        (leaf(&["localhost"], Some(&other_issuer), (-1, 30)), |p| {
            matches!(p, TlsProblem::Other(_))
        }),
    ];
    for (cert, expected) in cases {
        let server = serve(&cert);
        let problem = checked_problem(
            core.sync_server_trust_verified(&server.origin(), TrustChoice::Ca(ca_der.to_vec()))
                .expect("verified"),
        );
        assert!(expected(&problem), "{problem:?}");
        assert_eq!(
            core.sync_server_trust_info(&server.origin()).expect("info"),
            ServerTrust::System,
            "nothing stored"
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    }
    // A file that is no certificate is refused before any connection.
    let server = serve(&leaf(&["localhost"], Some(&issuer), (-1, 30)));
    let error = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::Ca(b"not a cert".to_vec()))
        .unwrap_err();
    assert!(
        matches!(
            error,
            shellpp_core::CoreError::Sync(HttpError::Refused("not_a_certificate"))
        ),
        "{error:?}"
    );
}

/// An added CA counts for its origin only: another origin served under the
/// same CA is still an unknown issuer, and an SSH host key is still new to
/// the app (TOFU): the CA has no part in it.
#[test]
fn an_added_ca_counts_for_its_origin_only_and_not_for_ssh() {
    let (ca_der, issuer) = ca();
    let first = serve(&leaf(&["localhost"], Some(&issuer), (-1, 30)));
    let second = serve(&leaf(&["localhost"], Some(&issuer), (-1, 30)));
    let unlocked = common::first_run();
    let core = &unlocked.core;
    core.sync_set_system_roots_for_test(Vec::new());
    let check = core
        .sync_server_trust_verified(&first.origin(), TrustChoice::Ca(ca_der.to_vec()))
        .expect("verified");
    assert!(matches!(check, ServerCheck::Ready { .. }));
    match checked_problem(core.sync_server_check(&second.origin()).expect("check")) {
        TlsProblem::UnknownIssuer(_) => {}
        other => panic!("the CA reached another origin: {other:?}"),
    }
    let mut blob = [0u8; 51];
    blob[..4].copy_from_slice(&11u32.to_be_bytes());
    blob[4..15].copy_from_slice(b"ssh-ed25519");
    blob[15..19].copy_from_slice(&32u32.to_be_bytes());
    blob[19..].copy_from_slice(&shellpp_core::crypto::random_array::<32>().expect("random"));
    let status = core
        .known_host_check(&shellpp_core::service::host_api::HostKeyCheck {
            address: "localhost".to_owned(),
            port: first.port,
            key_type: "ssh-ed25519".to_owned(),
            public_key: data_encoding::BASE64.encode(&blob),
        })
        .expect("host key check");
    assert_eq!(
        status,
        shellpp_core::service::host_api::HostKeyStatus::Unknown,
        "an SSH host key is asked about as before"
    );
}

/// The system's trust comes from the injected roots, never from the
/// machine's store: a certificate under the injected CA is trusted, a
/// self-signed one is not, and an empty set trusts nothing.
#[test]
fn the_systems_trust_comes_from_the_injected_roots() {
    let (system_ca, system_issuer) = ca();
    let trusted = serve(&leaf(&["localhost"], Some(&system_issuer), (-1, 30)));
    let self_signed = serve(&leaf(&["localhost"], None, (-1, 30)));
    let (_env, core) = core();
    core.sync_set_system_roots_for_test(vec![system_ca.to_vec()]);
    assert!(matches!(
        core.sync_server_check(&trusted.origin()).expect("check"),
        ServerCheck::Ready { .. }
    ));
    assert!(matches!(
        checked_problem(
            core.sync_server_check(&self_signed.origin())
                .expect("check")
        ),
        TlsProblem::UnknownIssuer(_)
    ));
    let empty = SystemRoots::Fixed(Arc::new(Vec::new()));
    let api =
        HttpApi::with_roots(&trusted.origin(), TrustMode::System, &empty, "0.7.0").expect("api");
    assert!(matches!(
        problem(api.server_info().map(|_| ())),
        Some(TlsProblem::UnknownIssuer(_))
    ));
}

/// The changed-certificate dialog (UI 12.2): "Trust the new certificate"
/// pins the certificate the server shows, never another one; "Use the
/// system's trust" removes the pin only when the system trusts the new
/// certificate. A refused choice keeps the old pin and sends nothing.
#[test]
fn a_changed_pin_is_confirmed_by_a_new_pin_or_the_systems_trust() {
    let (system_ca, system_issuer) = ca();
    let pinned = leaf(&["localhost"], None, (-1, 30));
    let server = serve(&pinned);
    let (_env, core) = core();
    core.sync_set_system_roots_for_test(vec![system_ca.to_vec()]);
    core.sync_server_trust(&server.origin(), TrustChoice::Pin(sha256(&pinned.der)))
        .expect("pinned");

    // Another self-signed certificate: the system does not trust it.
    let swapped = leaf(&["localhost"], None, (-1, 30));
    server.swap(&swapped);
    match checked_problem(core.sync_server_check(&server.origin()).expect("check")) {
        TlsProblem::PinChanged {
            pinned: old,
            new,
            system_trusts,
        } => {
            assert_eq!(old, sha256(&pinned.der));
            assert_eq!(new.sha256, sha256(&swapped.der));
            assert!(!system_trusts);
        }
        other => panic!("expected a changed pin, got {other:?}"),
    }
    let refused = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::System)
        .expect("verified");
    assert!(matches!(
        checked_problem(refused),
        TlsProblem::UnknownIssuer(_)
    ));
    let third = leaf(&["localhost"], None, (-1, 30));
    let other_pin = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::Pin(sha256(&third.der)))
        .expect("verified");
    assert!(matches!(
        checked_problem(other_pin),
        TlsProblem::PinChanged { .. }
    ));
    assert_eq!(
        core.sync_server_trust_info(&server.origin()).expect("info"),
        ServerTrust::Pinned {
            sha256: sha256(&pinned.der)
        },
        "a refused choice keeps the old pin"
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    let check = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::Pin(sha256(&swapped.der)))
        .expect("verified");
    assert!(matches!(check, ServerCheck::Ready { .. }));
    assert_eq!(
        core.sync_server_trust_info(&server.origin()).expect("info"),
        ServerTrust::Pinned {
            sha256: sha256(&swapped.der)
        }
    );

    // A certificate the system trusts: "Use the system's trust".
    let public = leaf(&["localhost"], Some(&system_issuer), (-1, 30));
    server.swap(&public);
    match checked_problem(core.sync_server_check(&server.origin()).expect("check")) {
        TlsProblem::PinChanged { system_trusts, .. } => assert!(system_trusts),
        other => panic!("expected a changed pin, got {other:?}"),
    }
    let check = core
        .sync_server_trust_verified(&server.origin(), TrustChoice::System)
        .expect("verified");
    assert!(matches!(check, ServerCheck::Ready { .. }));
    assert_eq!(
        core.sync_server_trust_info(&server.origin()).expect("info"),
        ServerTrust::System
    );
}

/// An unknown issuer is offered for trust only when the certificate's own
/// name and dates hold: a self-signed certificate for another name, or
/// out of its dates, is refused with that reason, so the dialog never
/// offers a pin the check would refuse (review of D187).
#[test]
fn an_unknown_issuer_with_another_name_or_bad_dates_is_refused_not_offered() {
    let (_env, core) = core();
    let wrong = serve(&leaf(&["sync.example.com"], None, (-1, 30)));
    assert!(matches!(
        checked_problem(core.sync_server_check(&wrong.origin()).expect("check")),
        TlsProblem::WrongName(_)
    ));
    let expired = serve(&leaf(&["localhost"], None, (-30, -1)));
    assert!(matches!(
        checked_problem(core.sync_server_check(&expired.origin()).expect("check")),
        TlsProblem::Expired(_)
    ));
}

/// A pinned origin that shows a new certificate for another name or out of
/// its dates is refused with that reason, never offered as a changed pin.
#[test]
fn a_changed_pin_to_a_bad_certificate_is_refused_not_offered() {
    let pinned = leaf(&["localhost"], None, (-1, 30));
    let server = serve(&pinned);
    let (_env, core) = core();
    core.sync_server_trust(&server.origin(), TrustChoice::Pin(sha256(&pinned.der)))
        .expect("pinned");
    server.swap(&leaf(&["localhost"], None, (-30, -1)));
    assert!(matches!(
        checked_problem(core.sync_server_check(&server.origin()).expect("check")),
        TlsProblem::Expired(_)
    ));
    server.swap(&leaf(&["sync.example.com"], None, (-1, 30)));
    assert!(matches!(
        checked_problem(core.sync_server_check(&server.origin()).expect("check")),
        TlsProblem::WrongName(_)
    ));
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

/// A CA file is read as PEM or as DER.
#[test]
fn ca_files_are_read_as_pem_or_der() {
    let (der, _) = ca();
    let pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        data_encoding::BASE64.encode(&der)
    );
    assert_eq!(
        shellpp_core::sync::tls::ca_der_from_file(pem.as_bytes()).as_deref(),
        Some(der.as_ref())
    );
    assert_eq!(
        shellpp_core::sync::tls::ca_der_from_file(&der).as_deref(),
        Some(der.as_ref())
    );
}
