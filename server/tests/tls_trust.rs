//! The certificate trust of a bound device against the real server behind a
//! TLS front (docs/sync-protocol.md, section 2.1; UI 12.2; D187): a changed
//! pinned certificate stops sync before any request reaches the server and
//! leaves the device signed in with its data; confirming the new
//! certificate, or the system's trust when the system trusts it, goes on
//! syncing without a new sign-in; until then (Cancel) sync stays stopped.
//! With an added CA a renewed certificate of that CA needs no question.
//!
//! Certificates are generated at run time; the "system's trust" of every
//! device is an injected root set, never the machine's store.

mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use shellpp_core::CoreError;
use shellpp_core::service::{ServerCheck, ServerTrust, SignInInput, SignInOutcome, TrustChoice};
use shellpp_core::sync::{HttpError, TlsProblem};
use support::*;

struct Cert {
    der: CertificateDer<'static>,
    key: Vec<u8>,
}

fn ca(name: &str) -> (Vec<u8>, Issuer<'static, KeyPair>) {
    let key = KeyPair::generate().expect("key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, name);
    let cert = params.self_signed(&key).expect("ca");
    (cert.der().to_vec(), Issuer::new(params, key))
}

fn leaf(issuer: Option<&Issuer<'static, KeyPair>>) -> Cert {
    let key = KeyPair::generate().expect("key");
    let mut params = CertificateParams::new(vec!["localhost".to_owned()]).expect("params");
    params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(30);
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

fn tls_config(cert: &Cert) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der.clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key.clone())),
            )
            .expect("tls config"),
    )
}

/// A TLS front on `https://localhost:<port>` that hands each request to the
/// test server (one request per connection) and counts what it forwarded.
/// [`Front::swap`] changes the certificate it shows.
struct Front {
    port: u16,
    config: Arc<Mutex<Arc<rustls::ServerConfig>>>,
    forwarded: Arc<AtomicUsize>,
}

impl Front {
    fn origin(&self) -> String {
        format!("https://localhost:{}", self.port)
    }

    fn swap(&self, cert: &Cert) {
        *self.config.lock().unwrap() = tls_config(cert);
    }

    fn forwarded(&self) -> usize {
        self.forwarded.load(Ordering::SeqCst)
    }
}

/// One HTTP/1.1 request from `stream`: the head and the body of its
/// `content-length`.
fn read_request(stream: &mut impl Read) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    Some((head, body))
}

fn front(backend: &str, cert: &Cert) -> Front {
    let backend: SocketAddr = backend
        .trim_start_matches("http://")
        .parse()
        .expect("backend address");
    let config = Arc::new(Mutex::new(tls_config(cert)));
    let forwarded = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (shown, counter) = (config.clone(), forwarded.clone());
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
                let Some((head, body)) = read_request(&mut tls) else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                // The same request with `connection: close`, so the answer
                // ends with the backend's connection.
                let mut lines: Vec<&str> = head
                    .trim_end()
                    .lines()
                    .filter(|line| {
                        !line
                            .split(':')
                            .next()
                            .is_some_and(|name| name.trim().eq_ignore_ascii_case("connection"))
                    })
                    .collect();
                lines.push("connection: close");
                let Ok(mut back) = TcpStream::connect(backend) else {
                    return;
                };
                let request = format!("{}\r\n\r\n", lines.join("\r\n"));
                if back.write_all(request.as_bytes()).is_err() || back.write_all(&body).is_err() {
                    return;
                }
                let mut answer = Vec::new();
                let _ = back.read_to_end(&mut answer);
                let _ = tls.write_all(&answer);
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    Front {
        port,
        config,
        forwarded,
    }
}

/// A device whose system's trust is `roots` (empty: trusts nothing).
fn device(name: &str, password: Option<&secrecy::SecretString>, roots: Vec<Vec<u8>>) -> Client {
    let client = match password {
        Some(password) => Client::local(name, password),
        None => Client::empty(name),
    };
    client.core.sync_set_system_roots_for_test(roots);
    client
}

fn input(origin: &str, email: &str, password: &secrecy::SecretString) -> SignInInput {
    SignInInput {
        origin: origin.to_owned(),
        email: email.to_owned(),
        password: password.clone(),
        registration_code: None,
    }
}

fn problem(check: ServerCheck) -> TlsProblem {
    match check {
        ServerCheck::Certificate { problem, .. } => problem,
        ServerCheck::Ready { .. } => panic!("expected a certificate problem"),
    }
}

fn ready(check: ServerCheck) {
    assert!(matches!(check, ServerCheck::Ready { .. }), "{check:?}");
}

/// The certificate code of a refused cycle.
fn cycle_error(client: &Client) -> String {
    match client.core.sync_now() {
        Err(CoreError::Sync(HttpError::Tls(problem))) => match *problem {
            TlsProblem::PinChanged { .. } => "certificate_changed".to_owned(),
            other => format!("{other:?}"),
        },
        other => panic!("expected a certificate refusal, got {other:?}"),
    }
}

/// A registers through a pinned self-signed certificate; the front swaps
/// it. Sync stops before any request, A stays signed in with its data and
/// its new edit pending; Cancel (doing nothing) keeps it stopped. "Trust the
/// new certificate" pins the shown one and the next cycle pushes the edit,
/// without signing in again; B restores everything through the new pin.
#[test]
fn a_changed_certificate_is_confirmed_without_signing_out() {
    let server = Server::start();
    let first = leaf(None);
    let front = front(&server.origin, &first);
    let origin = front.origin();
    let password = new_password();
    let email = email();
    let a = device("A", Some(&password), Vec::new());
    a.host("before.example.com");
    let info = match problem(a.core.sync_server_check(&origin).expect("check")) {
        TlsProblem::UnknownIssuer(info) => info,
        other => panic!("expected an unknown issuer, got {other:?}"),
    };
    ready(
        a.core
            .sync_server_trust_verified(&origin, TrustChoice::Pin(info.sha256))
            .expect("pinned"),
    );
    let status = a
        .core
        .sync_register(input(&origin, &email, &password))
        .expect("registered");
    assert!(status.bound && status.signed_in);

    let second = leaf(None);
    front.swap(&second);
    let sent = front.forwarded();
    a.host("during.example.com");
    assert_eq!(cycle_error(&a), "certificate_changed");
    let status = a.status();
    assert!(status.bound && status.signed_in, "not signed out");
    assert_eq!(status.last_error.as_deref(), Some("certificate_changed"));
    assert_eq!(status.pending, 1, "the edit waits");
    assert_eq!(
        a.addresses(),
        vec!["before.example.com", "during.example.com"]
    );
    // Cancel: nothing changes, sync stays stopped.
    assert_eq!(cycle_error(&a), "certificate_changed");
    assert_eq!(front.forwarded(), sent, "no request reached the server");

    // The dialog's details, then "Trust the new certificate".
    let new = match problem(a.core.sync_server_check(&origin).expect("check")) {
        TlsProblem::PinChanged {
            pinned,
            new,
            system_trusts,
        } => {
            assert_eq!(pinned, sha256(&first.der));
            assert!(!system_trusts);
            new
        }
        other => panic!("expected a changed pin, got {other:?}"),
    };
    assert_eq!(new.sha256, sha256(&second.der));
    ready(
        a.core
            .sync_server_trust_verified(&origin, TrustChoice::Pin(new.sha256))
            .expect("trusted"),
    );
    a.sync();
    let status = a.status();
    assert!(status.signed_in && status.last_error.is_none());
    assert_eq!(status.pending, 0, "the edit went up");

    let b = device("B", None, Vec::new());
    let info = match problem(b.core.sync_server_check(&origin).expect("check")) {
        TlsProblem::UnknownIssuer(info) => info,
        other => panic!("expected an unknown issuer, got {other:?}"),
    };
    assert_eq!(info.sha256, sha256(&second.der));
    ready(
        b.core
            .sync_server_trust_verified(&origin, TrustChoice::Pin(info.sha256))
            .expect("pinned"),
    );
    let outcome = b
        .core
        .sync_sign_in(input(&origin, &email, &password))
        .expect("restored");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    assert_eq!(
        b.addresses(),
        vec!["before.example.com", "during.example.com"]
    );
}

/// A pinned origin whose new certificate the system trusts: the dialog
/// offers "Use the system's trust", which removes the pin; sync goes on
/// without a new sign-in.
#[test]
fn the_systems_trust_replaces_a_changed_pin() {
    let server = Server::start();
    let (system_ca, system_issuer) = ca("Shell++ test public CA");
    let first = leaf(None);
    let front = front(&server.origin, &first);
    let origin = front.origin();
    let password = new_password();
    let a = device("A", Some(&password), vec![system_ca]);
    a.core
        .sync_server_trust(&origin, TrustChoice::Pin(sha256(&first.der)))
        .expect("pinned");
    a.core
        .sync_register(input(&origin, &email(), &password))
        .expect("registered");

    front.swap(&leaf(Some(&system_issuer)));
    a.host("after.example.com");
    assert_eq!(cycle_error(&a), "certificate_changed");
    match problem(a.core.sync_server_check(&origin).expect("check")) {
        TlsProblem::PinChanged { system_trusts, .. } => assert!(system_trusts),
        other => panic!("expected a changed pin, got {other:?}"),
    }
    ready(
        a.core
            .sync_server_trust_verified(&origin, TrustChoice::System)
            .expect("system"),
    );
    assert_eq!(
        a.core.sync_server_trust_info(&origin).expect("info"),
        ServerTrust::System
    );
    a.sync();
    let status = a.status();
    assert!(status.signed_in && status.last_error.is_none() && status.pending == 0);
}

/// With a CA added for the origin, a renewed certificate of that CA is
/// accepted without a question; one from elsewhere stops sync with the
/// trust choice again (an unknown issuer), the device still signed in.
#[test]
fn a_renewed_certificate_of_the_added_ca_needs_no_question() {
    let server = Server::start();
    let (home_ca, home_issuer) = ca("Shell++ test home CA");
    let front = front(&server.origin, &leaf(Some(&home_issuer)));
    let origin = front.origin();
    let password = new_password();
    let a = device("A", Some(&password), Vec::new());
    ready(
        a.core
            .sync_server_trust_verified(&origin, TrustChoice::Ca(home_ca))
            .expect("ca"),
    );
    a.core
        .sync_register(input(&origin, &email(), &password))
        .expect("registered");
    front.swap(&leaf(Some(&home_issuer)));
    a.host("renewed.example.com");
    a.sync();
    assert_eq!(a.status().pending, 0);

    front.swap(&leaf(None));
    a.host("stopped.example.com");
    match a.core.sync_now() {
        Err(CoreError::Sync(HttpError::Tls(problem))) => {
            assert!(
                matches!(*problem, TlsProblem::UnknownIssuer(_)),
                "{problem:?}"
            );
        }
        other => panic!("expected a refused certificate, got {other:?}"),
    }
    let status = a.status();
    assert!(status.signed_in && status.pending == 1);
    assert_eq!(status.last_error.as_deref(), Some("certificate_refused"));
}
