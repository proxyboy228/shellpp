//! The second device of the Android app's sync test on the emulator (M3,
//! `scripts\verify-android.ps1 -Emulator`): the real server behind a TLS
//! front whose certificate a CA made at run time issued, for `localhost`
//! (the phone reaches it at its own `localhost` through `adb reverse`: the
//! account's keys are bound to the server's origin, which must be the same
//! on both devices), and a desktop core client of the same account.
//!
//! Runs only with `SHELLPP_TEST_ANDROID_PEER=<folder>`; without it the test
//! returns at once. It writes `<folder>/peer.json` (the origin, the
//! account, the CA in PEM, the certificate's SHA-256) once the desktop has
//! registered and synced a host, then follows the script's steps through
//! files in the folder:
//!
//! 1. after `phone-1.done` (the phone signed in, found the desktop's host
//!    and added its own), the desktop syncs, checks the phone's host and adds
//!    a second one, then writes `desktop-1.done`;
//! 2. after `stop` it ends (the phone's later steps need no desktop step).
//!
//! A failure writes `failed` with its reason. Credentials and certificates
//! are made at run time.

mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::json;
use shellpp_core::service::TrustChoice;
use support::*;

const DESKTOP_HOST: &str = "desktop-host.example.com";
const DESKTOP_SECOND: &str = "desktop-second.example.com";
const PHONE_HOST: &str = "phone-host.example.org";

/// The longest wait for one step of the phone (an emulator test).
const STEP: Duration = Duration::from_secs(20 * 60);

fn tls_config(chain: CertificateDer<'static>, key: Vec<u8>) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![chain],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .expect("tls config"),
    )
}

/// One HTTP/1.1 request: the head and the body of its `content-length`.
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

/// A TLS front on 127.0.0.1 (the emulator's 10.0.2.2) that hands each
/// request to the test server, one request per connection; its port.
fn front(backend: &str, config: Arc<rustls::ServerConfig>) -> u16 {
    let backend: SocketAddr = backend
        .trim_start_matches("http://")
        .parse()
        .expect("backend address");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let config = config.clone();
            thread::spawn(move || {
                let Ok(mut conn) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::Stream::new(&mut conn, &mut stream);
                let Some((head, body)) = read_request(&mut tls) else {
                    return;
                };
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
    port
}

/// Waits for `<folder>/<name>` (the script's step), or for `stop`.
fn wait_for(folder: &Path, name: &str) -> bool {
    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        if folder.join(name).exists() {
            return true;
        }
        if folder.join("stop").exists() {
            return false;
        }
        thread::sleep(Duration::from_millis(500));
    }
    false
}

fn fail(folder: &Path, reason: &str) -> ! {
    let _ = std::fs::write(folder.join("failed"), reason);
    panic!("{reason}");
}

#[test]
fn the_desktop_side_of_the_android_sync_test() {
    let Some(folder) = std::env::var_os("SHELLPP_TEST_ANDROID_PEER").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&folder).expect("folder");

    // A CA and the server's certificate for every name the two sides use.
    let ca_key = KeyPair::generate().expect("key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Shell++ Android test CA");
    let ca_cert = ca_params.self_signed(&ca_key).expect("ca");
    let ca_der = ca_cert.der().to_vec();
    let ca_pem = {
        let body = data_encoding::BASE64.encode(ca_cert.der());
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(64)
            .map(|line| std::str::from_utf8(line).expect("base64"))
            .collect();
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            lines.join("\n")
        )
    };
    let issuer = Issuer::new(ca_params, ca_key);
    let leaf_key = KeyPair::generate().expect("key");
    let mut leaf_params = CertificateParams::new(vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "10.0.2.2".to_owned(),
    ])
    .expect("params");
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    let now = time::OffsetDateTime::now_utc();
    leaf_params.not_before = now - time::Duration::days(1);
    leaf_params.not_after = now + time::Duration::days(30);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).expect("leaf");
    let leaf_sha256 = shellpp_core::sync::tls::sha256(leaf.der());

    let server = Server::start();
    let port = front(
        &server.origin,
        tls_config(leaf.der().clone(), leaf_key.serialize_der()),
    );

    // The desktop: an account with a host, its CA in the "system's" roots.
    let password = new_password();
    let email = email();
    let desktop = Client::local("Desktop", &password);
    desktop.core.sync_set_system_roots_for_test(vec![ca_der]);
    let origin = format!("https://localhost:{port}");
    let check = desktop.core.sync_server_check(&origin).expect("check");
    assert!(
        matches!(check, shellpp_core::service::ServerCheck::Ready { .. }),
        "{check:?}"
    );
    desktop
        .core
        .sync_server_trust(&origin, TrustChoice::System)
        .expect("trust");
    desktop.host(DESKTOP_HOST);
    let status = desktop
        .core
        .sync_register(shellpp_core::service::SignInInput {
            origin: origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        })
        .expect("registered");
    assert!(status.bound);
    desktop.sync();

    let peer = json!({
        // The same origin on both sides (the account's keys are bound to
        // it): the script forwards the phone's port to this one (adb reverse).
        "origin": origin,
        "email": email,
        "password": expose(&password),
        "ca_pem": ca_pem,
        "sha256": shellpp_core::sync::fingerprint_text(&leaf_sha256),
        "desktop_host": DESKTOP_HOST,
        "desktop_second": DESKTOP_SECOND,
        "phone_host": PHONE_HOST,
    });
    let temporary = folder.join("peer.json.tmp");
    std::fs::write(&temporary, peer.to_string()).expect("peer.json");
    std::fs::rename(&temporary, folder.join("peer.json")).expect("peer.json");

    // Step 1: the phone's host reaches the desktop, the desktop's second host goes up.
    if !wait_for(&folder, "phone-1.done") {
        fail(&folder, "the phone's first step did not come");
    }
    desktop.sync();
    if !desktop
        .addresses()
        .iter()
        .any(|address| address == PHONE_HOST)
    {
        fail(&folder, "the phone's host did not reach the desktop");
    }
    desktop.host(DESKTOP_SECOND);
    desktop.sync();
    std::fs::write(folder.join("desktop-1.done"), "").expect("step");

    // The phone's later steps (a sync on resume, a restore on a clean
    // install) need the server only.
    let deadline = Instant::now() + STEP * 2;
    while !folder.join("stop").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
    }
    drop(server);
}
