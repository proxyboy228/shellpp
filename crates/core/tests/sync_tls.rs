//! Certificate trust of the sync client against a real TLS server
//! (docs/sync-protocol.md, section 2.1; D93; plan run 7, section 4,
//! "certificate swap"): the system roots, a pinned certificate, a CA added
//! for one origin; a changed pin stops the request before any byte of it
//! is sent; expired and wrong-name certificates are refused even when
//! pinned. Certificates are generated at run time.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
/// requests it actually received.
struct TlsServer {
    port: u16,
    requests: Arc<AtomicUsize>,
}

fn serve(cert: &Cert) -> TlsServer {
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
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let config = config.clone();
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
    TlsServer { port, requests }
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
    match check(&server, TrustMode::System) {
        Err(HttpError::Tls(TlsProblem::UnknownIssuer(info))) => {
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
    match check(&server, TrustMode::Pinned(sha256(&pinned.der))) {
        Err(HttpError::Tls(TlsProblem::PinChanged {
            pinned: old,
            new,
            system_trusts,
        })) => {
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
    match check(&server, TrustMode::Pinned(sha256(&expired.der))) {
        Err(HttpError::Tls(TlsProblem::Expired(_))) => {}
        other => panic!("expected expired, got {other:?}"),
    }
    let wrong = leaf(&["sync.example.com"], None, (-1, 30));
    let server = serve(&wrong);
    match check(&server, TrustMode::Pinned(sha256(&wrong.der))) {
        Err(HttpError::Tls(TlsProblem::WrongName(_))) => {}
        other => panic!("expected wrong name, got {other:?}"),
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
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
