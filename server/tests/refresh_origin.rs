//! The refresh token goes only to the server that issued it (storage-format,
//! section 6.4; run 8 agent C finding S5; D188): its wrap is bound to the
//! normalized origin, so a `server_account.server_url` changed in the local
//! database (by someone with write access to it, who also controls the new
//! address) never receives the token: the device is signed out and signs in
//! again with the master password.

mod support;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use shellpp_core::platform::OsKind;
use shellpp_core::{Core, CoreConfig};
use support::*;

const INFO: &str = r#"{"version":"0.7.0","api_versions":["v1"],"registration":"open","smtp":false,"limits":{"max_envelope_bytes":262232,"max_push_items":500,"max_push_body_bytes":16777216,"default_pull_limit":500,"max_pull_limit":1000,"tombstone_retention_days":90},"server_generation":null}"#;

/// A plain-HTTP server that records every request it gets and answers
/// `GET /v1/server` (anything else `401`).
fn recorder() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = log.clone();
            thread::spawn(move || {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                // The head, then whatever body arrives within a moment.
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(300)));
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buffer).into_owned();
                let info = text.starts_with("GET /v1/server ");
                log.lock().unwrap().push(text);
                let answer = if info {
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{INFO}",
                        INFO.len()
                    )
                } else {
                    let body = r#"{"error":{"code":"unauthenticated","message":"x"}}"#;
                    format!(
                        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(answer.as_bytes());
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

#[test]
fn a_changed_server_address_never_receives_the_refresh_token() {
    let server = Server::start();
    let password = new_password();
    let a = Client::local("A", &password);
    a.host("kept.example.com");
    a.register(&server, &email(), &password);
    a.sync();

    // Someone with write access to the local database points the account
    // at a server of theirs.
    let (other, seen) = recorder();
    let Client { dir, mem, core } = a;
    drop(core);
    let db = rusqlite::Connection::open(dir.path().join("data").join("shellpp.db")).expect("db");
    let changed = db
        .execute("UPDATE server_account SET server_url = ?1", [&other])
        .expect("server_url changed");
    assert_eq!(changed, 1);
    drop(db);

    let core = Core::open(
        CoreConfig {
            data_dir: dir.path().join("data"),
            app_version: APP_VERSION.to_owned(),
        },
        mem.platform(OsKind::Windows, "A"),
    )
    .expect("core opens");
    core.unlock(password.clone()).expect("unlocked");
    let result = core.sync_now();
    assert!(result.is_err(), "{result:?}");
    let status = core.sync_status().expect("status");
    assert!(
        status.bound && !status.signed_in,
        "signed out: sign in again"
    );

    let requests = seen.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("shellpp_rt_") && !request.contains("token/refresh")),
        "the token reached another server: {requests:?}"
    );
    // Nothing local was lost.
    let client = Client { dir, mem, core };
    assert_eq!(client.addresses(), vec!["kept.example.com"]);
}
