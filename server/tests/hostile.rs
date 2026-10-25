//! A hostile server (plan run 7, section 4; docs/sync-protocol.md, sections
//! 6.4 and 6.9; principle 3): a proxy between the client and the real
//! server rewrites the pull answers. Whatever it sends, the client accepts
//! nothing it cannot authenticate, loses no local record and reports the
//! problem (integrity warnings, "server state rolled back").

mod support;

use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD;
use serde_json::Value;
use shellpp_core::CoreError;
use shellpp_core::service::SignInOutcome;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, PartialEq)]
enum Mode {
    Pass,
    /// The envelopes of the first two records trade places.
    Swap,
    /// One ciphertext byte of every envelope is flipped.
    Tamper,
    /// Every envelope seen so far is sent again (older revisions).
    Replay,
    /// No records, the cursor still advances.
    Withhold,
    /// An envelope of another account's vault is added.
    Foreign(String),
    /// `current_epoch` below the device's.
    LowerEpoch,
    /// The header of the first record names key epoch 7 (the server holds
    /// epoch 1).
    ForgeEpoch,
    /// The header of the first record names key epoch 2 and the page says
    /// the vault's current epoch is 2 (a rotation the device has not seen).
    NewerEpoch,
    /// The login's answer carries a personal `kek_wrap` with one byte
    /// flipped (it opens to nothing).
    TamperLoginWrap,
    /// Every pull is answered `503` (the server is busy).
    FailPull,
    /// Push answers carry no results (the server's write is unknown).
    EmptyPush,
    /// `ForgeEpoch` on the first page of a pull from 0, and the first pull
    /// of a later page fails with `500` (`failed` once it did).
    ForgeThenFail {
        failed: bool,
    },
    /// Forces a full resync (`410 purged`), leaves `hide` out of the
    /// snapshot and answers its push-back with the older `stale` version.
    Purge {
        hide: [u8; 16],
        stale: String,
        sent_410: bool,
    },
}

struct Proxy {
    origin: String,
    mode: Arc<Mutex<Mode>>,
    seen: Arc<Mutex<Vec<Value>>>,
    /// Push requests passed on.
    pushes: Arc<std::sync::atomic::AtomicUsize>,
    login_gate: Arc<LoginGate>,
}

/// Holds the login's answer (the server has issued its tokens) until the
/// test releases it ([`Proxy::hold_login`]).
#[derive(Default)]
struct LoginGate {
    hold: std::sync::atomic::AtomicBool,
    held: (Mutex<bool>, std::sync::Condvar),
    release: tokio::sync::Notify,
}

impl Proxy {
    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    /// The next login's answer waits for [`Proxy::release_login`].
    fn hold_login(&self) {
        self.login_gate
            .hold
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Waits until a login's answer is held.
    fn wait_login_held(&self) {
        let (held, signal) = &self.login_gate.held;
        let guard = held.lock().unwrap();
        let (_guard, wait) = signal
            .wait_timeout_while(guard, std::time::Duration::from_secs(60), |held| !*held)
            .unwrap();
        assert!(!wait.timed_out(), "a login reached the proxy");
    }

    fn release_login(&self) {
        self.login_gate
            .hold
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.login_gate.release.notify_one();
    }

    /// Envelopes (base64url) the proxy saw in pull answers.
    fn seen(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }
}

/// The record id in an envelope's header.
fn record_id(record: &Value) -> Option<[u8; 16]> {
    let bytes = BASE64URL_NOPAD
        .decode(record["envelope"].as_str()?.as_bytes())
        .ok()?;
    shellpp_sync_api::envelope::split(&bytes)
        .ok()
        .map(|parts| parts.header.record_id)
}

/// A push answer whose `conflict` for `hide` carries `stale` as `current`.
fn rewrite_push(mode: &Mode, body: &[u8]) -> Option<Vec<u8>> {
    if *mode == Mode::EmptyPush {
        return Some(br#"{"results":[]}"#.to_vec());
    }
    let Mode::Purge { hide, stale, .. } = mode else {
        return None;
    };
    if stale.is_empty() {
        return None; // Only the forced resync.
    }
    let mut answer: Value = serde_json::from_slice(body).ok()?;
    let hidden = uuid::Uuid::from_bytes(*hide).to_string();
    for result in answer.get_mut("results")?.as_array_mut()? {
        if result["record_id"].as_str() == Some(hidden.as_str()) && result["status"] == "conflict" {
            result["current"] = serde_json::json!({"seq": 1, "envelope": stale});
        }
    }
    serde_json::to_vec(&answer).ok()
}

fn rewrite(mode: &Mode, seen: &Mutex<Vec<Value>>, body: &[u8]) -> Option<Vec<u8>> {
    let mut page: Value = serde_json::from_slice(body).ok()?;
    let records = page.get_mut("records")?.as_array_mut()?;
    seen.lock().unwrap().extend(records.iter().cloned());
    match mode {
        Mode::Pass | Mode::EmptyPush => return None,
        Mode::Swap => {
            if records.len() >= 2 {
                let first = records[0]["envelope"].clone();
                records[0]["envelope"] = records[1]["envelope"].clone();
                records[1]["envelope"] = first;
            }
        }
        Mode::Tamper => {
            for record in records.iter_mut() {
                let text = record["envelope"].as_str()?.to_owned();
                let mut bytes = BASE64URL_NOPAD.decode(text.as_bytes()).ok()?;
                let last = bytes.len() - 1;
                bytes[last] ^= 0x01;
                record["envelope"] = Value::String(BASE64URL_NOPAD.encode(&bytes));
            }
        }
        Mode::Replay => {
            let old = seen.lock().unwrap().clone();
            let next = page["next_cursor"].as_u64().unwrap_or(0);
            let replayed: Vec<Value> = old
                .into_iter()
                .enumerate()
                .map(|(i, mut record)| {
                    record["seq"] = Value::from(next + 1 + i as u64);
                    record
                })
                .collect();
            let count = replayed.len() as u64;
            *page.get_mut("records")? = Value::Array(replayed);
            page["next_cursor"] = Value::from(next + count);
        }
        Mode::Withhold => records.clear(),
        Mode::Foreign(envelope) => {
            let seq = page["next_cursor"].as_u64().unwrap_or(0) + 1;
            page["records"]
                .as_array_mut()?
                .push(serde_json::json!({"seq": seq, "envelope": envelope}));
            page["next_cursor"] = Value::from(seq);
        }
        Mode::LowerEpoch => page["current_epoch"] = Value::from(0),
        Mode::ForgeEpoch | Mode::ForgeThenFail { .. } => {
            if let Some(record) = records.first_mut() {
                let text = record["envelope"].as_str()?.to_owned();
                let mut bytes = BASE64URL_NOPAD.decode(text.as_bytes()).ok()?;
                bytes[44..48].copy_from_slice(&7u32.to_be_bytes());
                record["envelope"] = Value::String(BASE64URL_NOPAD.encode(&bytes));
            }
        }
        Mode::NewerEpoch => {
            if let Some(record) = records.first_mut() {
                let text = record["envelope"].as_str()?.to_owned();
                let mut bytes = BASE64URL_NOPAD.decode(text.as_bytes()).ok()?;
                bytes[44..48].copy_from_slice(&2u32.to_be_bytes());
                record["envelope"] = Value::String(BASE64URL_NOPAD.encode(&bytes));
            }
            page["current_epoch"] = Value::from(2);
        }
        Mode::Purge { hide, .. } => {
            records.retain(|record| record_id(record) != Some(*hide));
        }
        Mode::TamperLoginWrap | Mode::FailPull => return None,
    }
    serde_json::to_vec(&page).ok()
}

/// The login's answer with its personal `kek_wrap` changed
/// ([`Mode::TamperLoginWrap`]).
fn rewrite_login(mode: &Mode, body: &[u8]) -> Option<Vec<u8>> {
    if !matches!(mode, Mode::TamperLoginWrap) {
        return None;
    }
    let mut answer: Value = serde_json::from_slice(body).ok()?;
    let wrap = answer.pointer_mut("/account/personal_vault/kek_wrap")?;
    let mut bytes = BASE64URL_NOPAD.decode(wrap.as_str()?.as_bytes()).ok()?;
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    *wrap = Value::String(BASE64URL_NOPAD.encode(&bytes));
    serde_json::to_vec(&answer).ok()
}

async fn read_message(stream: &mut tokio::net::TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    Some((head, buffer[head_end..].to_vec()))
}

/// The head without `content-length` and `connection`, plus ours.
fn with_length(head: &str, length: usize) -> String {
    let mut lines: Vec<&str> = head
        .trim_end_matches("\r\n\r\n")
        .split("\r\n")
        .filter(|line| {
            let name = line
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            name != "content-length" && name != "connection"
        })
        .collect();
    let length_line = format!("content-length: {length}");
    lines.push(&length_line);
    lines.push("connection: close");
    format!("{}\r\n\r\n", lines.join("\r\n"))
}

fn proxy(server: &Server) -> Proxy {
    let upstream = server.config.bind;
    let mode = Arc::new(Mutex::new(Mode::Pass));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = server
        .runtime()
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let pushes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let login_gate = Arc::new(LoginGate::default());
    let task_gate = login_gate.clone();
    let (task_mode, task_seen, task_pushes) = (mode.clone(), seen.clone(), pushes.clone());
    server.runtime().spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                return;
            };
            let (mode, seen, pushes) = (task_mode.clone(), task_seen.clone(), task_pushes.clone());
            let gate = task_gate.clone();
            tokio::spawn(async move {
                let Some((head, body)) = read_message(&mut client).await else {
                    return;
                };
                let pull = head.starts_with("GET /v1/vaults/") && head.contains("/changes");
                let push = head.starts_with("POST /v1/vaults/") && head.contains("/changes");
                let login = head.starts_with("POST /v1/auth/login");
                if push {
                    pushes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                // The forced resync: one `410 purged` for the first pull.
                let send_410 = pull && {
                    let mut current = mode.lock().unwrap();
                    match &mut *current {
                        Mode::Purge { sent_410, .. } if !*sent_410 => {
                            *sent_410 = true;
                            true
                        }
                        _ => false,
                    }
                };
                // The first pull of a later page fails once.
                let from_zero = head.contains("since=0&") || head.contains("since=0 ");
                let send_500 = pull && !from_zero && {
                    let mut current = mode.lock().unwrap();
                    match &mut *current {
                        Mode::ForgeThenFail { failed } if !*failed => {
                            *failed = true;
                            true
                        }
                        _ => false,
                    }
                };
                if pull && *mode.lock().unwrap() == Mode::FailPull {
                    let body = r#"{"error":{"code":"unavailable","message":"busy"}}"#;
                    let head = format!(
                        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = client.write_all(head.as_bytes()).await;
                    let _ = client.write_all(body.as_bytes()).await;
                    let _ = client.shutdown().await;
                    return;
                }
                if send_500 {
                    let body = r#"{"error":{"code":"internal","message":"internal"}}"#;
                    let head = format!(
                        "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = client.write_all(head.as_bytes()).await;
                    let _ = client.write_all(body.as_bytes()).await;
                    let _ = client.shutdown().await;
                    return;
                }
                {
                    if send_410 {
                        let body = r#"{"error":{"code":"cursor_expired","message":"cursor expired","details":{"reason":"purged"}}}"#;
                        let head = format!(
                            "HTTP/1.1 410 Gone
content-type: application/json
content-length: {}
connection: close

",
                            body.len()
                        );
                        let _ = client.write_all(head.as_bytes()).await;
                        let _ = client.write_all(body.as_bytes()).await;
                        let _ = client.shutdown().await;
                        return;
                    }
                }
                let Ok(mut up) = tokio::net::TcpStream::connect(upstream).await else {
                    return;
                };
                let request = with_length(&head, body.len());
                if up.write_all(request.as_bytes()).await.is_err()
                    || up.write_all(&body).await.is_err()
                {
                    return;
                }
                let Some((response_head, mut response)) = read_message(&mut up).await else {
                    return;
                };
                if pull && response_head.starts_with("HTTP/1.1 200") {
                    let current = mode.lock().unwrap().clone();
                    if let Some(changed) = rewrite(&current, &seen, &response) {
                        response = changed;
                    }
                }
                if login && response_head.starts_with("HTTP/1.1 200") {
                    let current = mode.lock().unwrap().clone();
                    if let Some(changed) = rewrite_login(&current, &response) {
                        response = changed;
                    }
                    if gate.hold.load(std::sync::atomic::Ordering::SeqCst) {
                        {
                            let (held, signal) = &gate.held;
                            *held.lock().unwrap() = true;
                            signal.notify_all();
                        }
                        gate.release.notified().await;
                    }
                }
                if push && response_head.starts_with("HTTP/1.1 200") {
                    let current = mode.lock().unwrap().clone();
                    if let Some(changed) = rewrite_push(&current, &response) {
                        response = changed;
                    }
                }
                let head = with_length(&response_head, response.len());
                let _ = client.write_all(head.as_bytes()).await;
                let _ = client.write_all(&response).await;
                let _ = client.shutdown().await;
            });
        }
    });
    Proxy {
        origin: format!("http://127.0.0.1:{port}"),
        mode,
        seen,
        pushes,
        login_gate,
    }
}

/// A and B on one account through the proxy; A has two hosts.
fn setup(server: &Server) -> (Proxy, Client, Client, String, secrecy::SecretString) {
    let proxy = proxy(server);
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("one.example.com");
    a.host("two.example.com");
    a.core
        .sync_register(shellpp_core::service::SignInInput {
            origin: proxy.origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        })
        .expect("registered");
    let b = Client::empty("B");
    let outcome = b
        .core
        .sync_sign_in(shellpp_core::service::SignInInput {
            origin: proxy.origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        })
        .expect("signed in");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    (proxy, a, b, email, password)
}

fn edit_both(a: &Client) {
    let hosts = a.views(&[shellpp_core::service::dto::RecordType::Host]);
    for view in hosts {
        let id = view.record_id();
        let mut data = a.data(id);
        data["label"] = Value::String("edited on A".into());
        a.save(Some(id), shellpp_core::service::dto::RecordType::Host, data);
    }
    a.sync();
}

fn labels(client: &Client) -> Vec<String> {
    let mut list: Vec<String> = client
        .views(&[shellpp_core::service::dto::RecordType::Host])
        .iter()
        .map(|view| {
            client.data(view.record_id())["label"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    list.sort();
    list
}

#[test]
fn swapped_envelopes_stay_with_their_records_and_tampered_ones_are_refused() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    // Swapped between two records: each envelope names its own record in
    // the authenticated header, so the swap moves nothing.
    edit_both(&a);
    proxy.set(Mode::Swap);
    let report = b.sync();
    assert_eq!(report.integrity_warnings, 0);
    assert_eq!(labels(&b), vec!["edited on A", "edited on A"]);
    let addresses = b.addresses();
    assert_eq!(addresses, vec!["one.example.com", "two.example.com"]);
    // A flipped byte: refused, reported, nothing applied.
    proxy.set(Mode::Pass);
    let id = a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id();
    let mut data = a.data(id);
    data["label"] = Value::String("second edit".into());
    a.save(Some(id), shellpp_core::service::dto::RecordType::Host, data);
    a.sync();
    proxy.set(Mode::Tamper);
    let before = labels(&b);
    let report = b.sync();
    assert!(report.integrity_warnings >= 1, "reported");
    assert_eq!(labels(&b), before, "nothing forged was applied");
    assert!(b.status().integrity_warnings >= 1, "the status counts it");
    assert_eq!(b.addresses(), vec!["one.example.com", "two.example.com"]);
}

#[test]
fn replayed_older_versions_change_nothing() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    edit_both(&a);
    b.sync();
    assert_eq!(labels(&b), vec!["edited on A", "edited on A"]);
    proxy.set(Mode::Replay);
    b.sync();
    assert_eq!(
        labels(&b),
        vec!["edited on A", "edited on A"],
        "no rollback"
    );
    assert!(!proxy.seen().is_empty());
}

#[test]
fn withheld_records_delete_nothing() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    let id = a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id();
    a.delete(id);
    a.sync();
    proxy.set(Mode::Withhold);
    b.sync();
    assert_eq!(
        b.addresses(),
        vec!["one.example.com", "two.example.com"],
        "absence never deletes"
    );
}

#[test]
fn an_envelope_of_another_vault_is_refused() {
    let server = Server::start();
    let (proxy, _a, b, _, _) = setup(&server);
    // A second account on the same server: its envelopes are genuine, but
    // of another vault.
    let other_password = new_password();
    let c = Client::local("C", &other_password);
    c.host("intruder.example.com");
    c.core
        .sync_register(shellpp_core::service::SignInInput {
            origin: proxy.origin.clone(),
            email: email(),
            password: other_password,
            registration_code: None,
        })
        .expect("registered");
    // C's own pull goes through the proxy last: its envelopes are C's.
    c.sync();
    let foreign = proxy
        .seen()
        .last()
        .and_then(|record| record["envelope"].as_str().map(str::to_owned))
        .expect("an envelope of C's vault");
    proxy.set(Mode::Foreign(foreign));
    let report = b.sync();
    assert!(report.integrity_warnings >= 1);
    assert_eq!(b.addresses(), vec!["one.example.com", "two.example.com"]);
}

#[test]
fn a_lower_epoch_stops_sync_without_changes() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    edit_both(&a);
    let before = labels(&b);
    proxy.set(Mode::LowerEpoch);
    // Personal epoch 1 on the device; the proxy claims 0.
    match b.core.sync_now() {
        Err(CoreError::Sync(error)) => assert_eq!(error.code(), "rolled_back"),
        other => panic!("expected rolled_back, got {other:?}"),
    }
    assert_eq!(labels(&b), before);
    assert_eq!(b.status().last_error.as_deref(), Some("rolled_back"));
    proxy.set(Mode::Pass);
    b.sync();
    assert_eq!(labels(&b), vec!["edited on A", "edited on A"]);
}

/// Review finding (principle 3): a forced resync leaves a record out, its
/// push-back meets an older genuine version as `current`. The older
/// version never replaces the local one, and no copy is made of it.
#[test]
fn a_stale_current_after_a_forced_resync_is_not_adopted() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    let id = a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id();
    // The version B saw first: the stale one to replay.
    let stale = proxy
        .seen()
        .into_iter()
        .find(|record| record_id(record) == Some(*id.as_bytes()))
        .and_then(|record| record["envelope"].as_str().map(str::to_owned))
        .expect("the first version");
    let mut data = a.data(id);
    data["label"] = Value::String("current".into());
    a.save(Some(id), shellpp_core::service::dto::RecordType::Host, data);
    a.sync();
    b.sync();
    assert_eq!(b.data(id)["label"], "current");
    proxy.set(Mode::Purge {
        hide: *id.as_bytes(),
        stale,
        sent_410: false,
    });
    let _ = b.core.sync_now();
    let _ = b.core.sync_now();
    assert_eq!(
        b.data(id)["label"],
        "current",
        "the stale version was refused"
    );
    assert!(b.core.sync_conflicts().expect("conflicts").is_empty());
    assert_eq!(b.addresses(), vec!["one.example.com", "two.example.com"]);
}

/// A forced resync whose snapshot lacks a synced record pushes it back and
/// lists it in "Brought back"; "Delete again" deletes it everywhere.
#[test]
fn a_forced_resync_lists_what_it_brought_back_and_delete_again_works() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    let id = a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id();
    proxy.set(Mode::Purge {
        hide: *id.as_bytes(),
        stale: String::new(),
        sent_410: false,
    });
    let _ = b.core.sync_now();
    let back = b.core.sync_brought_back().expect("brought back");
    assert_eq!(back.len(), 1, "the hidden record is listed");
    assert_eq!(back[0].record_id, id);
    proxy.set(Mode::Pass);
    b.core
        .sync_brought_back_delete(&[id])
        .expect("delete again");
    assert!(b.core.sync_brought_back().expect("list").is_empty());
    b.sync();
    a.sync();
    assert_eq!(a.addresses().len(), 1, "deleted on A too");
    assert!(
        a.core
            .recently_deleted_list()
            .expect("trash")
            .iter()
            .any(|item| item.record_id == id),
        "and kept in A's Recently deleted"
    );
}

/// Run 7 finding 7 (sections 6.4 and 6.9; D173, item 1.7): a header that
/// names a key epoch the server does not hold stops the vault's sync with
/// `forged_epoch`, again in every cycle, and changes nothing; the user's
/// "Reset sync state of this vault" resyncs in full, refuses that record
/// (never its epoch), keeps every local record and lets sync go on.
#[test]
fn a_forged_key_epoch_stops_sync_until_the_user_resets_it() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    edit_both(&a);
    let before = labels(&b);
    proxy.set(Mode::ForgeEpoch);
    for _ in 0..2 {
        match b.core.sync_now() {
            Err(CoreError::Sync(error)) => assert_eq!(error.code(), "forged_epoch"),
            other => panic!("expected forged_epoch, got {other:?}"),
        }
        assert_eq!(b.status().last_error.as_deref(), Some("forged_epoch"));
        assert!(!b.status().newer_epoch, "not a newer epoch to sign in for");
    }
    assert_eq!(labels(&b), before, "nothing applied from the forged page");

    let report = b.core.sync_reset_vault_state().expect("reset");
    assert!(
        report.integrity_warnings >= 1,
        "the forged record is refused"
    );
    let status = b.status();
    assert_eq!(status.last_error, None);
    assert_eq!(b.addresses(), vec!["one.example.com", "two.example.com"]);
    // The epoch was never taken: the next cycles run, and once the server
    // answers honestly both devices agree again.
    proxy.set(Mode::Pass);
    b.sync();
    b.sync();
    assert_eq!(b.status().last_error, None);
    assert_eq!(labels(&b), vec!["edited on A", "edited on A"]);
    let mut edited =
        a.data(a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id());
    edited["label"] = Value::String("after the reset".into());
    a.save(
        Some(a.views(&[shellpp_core::service::dto::RecordType::Host])[0].record_id()),
        shellpp_core::service::dto::RecordType::Host,
        edited,
    );
    a.sync();
    b.sync();
    assert!(labels(&b).contains(&"after the reset".to_owned()));
}

/// Cross review of run 9 (section 6.5): push answers without results leave
/// the items pending. A cycle sends each of them once, not the same batch
/// up to the batch limit: 600 items go in two requests.
#[test]
fn items_left_pending_by_a_push_are_sent_once_per_cycle() {
    let server = Server::start();
    let (proxy, a, _b, _, _) = setup(&server);
    for i in 0..600 {
        a.host(&format!("bulk{i}.example.com"));
    }
    proxy.set(Mode::EmptyPush);
    let before = proxy.pushes.load(std::sync::atomic::Ordering::SeqCst);
    let _ = a.core.sync_now();
    let sent = proxy.pushes.load(std::sync::atomic::Ordering::SeqCst) - before;
    assert!(sent <= 2, "{sent} pushes in one cycle");
    assert!(a.status().pending >= 600, "nothing is taken as synced");
    proxy.set(Mode::Pass);
    a.sync();
    assert_eq!(a.status().pending, 0);
}

/// Cross review of run 9 (D183): a reset whose resync is cut off after the
/// forged page (a later page fails) goes on refusing that record in the
/// next cycle, until its resync completes; the stop does not come back.
#[test]
fn an_interrupted_reset_still_refuses_the_forged_epoch() {
    let server = Server::start();
    let (proxy, a, b, _, _) = setup(&server);
    // More than one page (500 records each).
    for i in 0..600 {
        a.host(&format!("page{i}.example.com"));
    }
    a.sync();
    b.sync();
    edit_both(&a);
    proxy.set(Mode::ForgeEpoch);
    assert!(b.core.sync_now().is_err(), "the forged epoch stops sync");
    assert_eq!(b.status().last_error.as_deref(), Some("forged_epoch"));

    proxy.set(Mode::ForgeThenFail { failed: false });
    assert!(
        b.core.sync_reset_vault_state().is_err(),
        "the second page fails"
    );
    // The next cycle continues the reset's resync: the forged record is
    // refused again, not a new stop.
    let report = b.core.sync_now().expect("the resync completes");
    assert!(report.integrity_warnings >= 1);
    assert_eq!(b.status().last_error, None);
    // Once it is complete, a forged header stops sync again.
    proxy.set(Mode::Pass);
    edit_both(&a);
    proxy.set(Mode::ForgeEpoch);
    assert!(b.core.sync_now().is_err());
    assert_eq!(b.status().last_error.as_deref(), Some("forged_epoch"));
}

/// Live refresh tokens and revoked devices of the account with `email`.
fn sessions_and_revoked_devices(server: &Server, email: &str) -> (i64, i64) {
    let count = |sql: &str| {
        server
            .db
            .tx(shellpp_server::db::TxKind::Read, |tx| {
                let rows = tx.query(
                    sql,
                    &[shellpp_server::db::Val::from(email)],
                    &[shellpp_server::db::Ty::Int],
                )?;
                Ok::<_, shellpp_server::db::DbError>(rows[0][0].int().unwrap_or(0))
            })
            .expect("count")
    };
    (
        count(
            "SELECT COUNT(*) FROM refresh_tokens t JOIN users u ON u.user_id = t.user_id \
             WHERE u.email = ? AND t.revoked_at IS NULL",
        ),
        count(
            "SELECT COUNT(*) FROM devices d JOIN users u ON u.user_id = d.user_id \
             WHERE u.email = ? AND d.revoked_at IS NOT NULL",
        ),
    )
}

/// Run 13, plan 3.9 (D209 for every refusal, D216): a device with local data
/// signs in through a hostile server; the login goes through, the merge or
/// the key wraps refuse with `code` before the device is bound, and what
/// that login opened is revoked: no token of it stays live and its device
/// row is marked revoked. Nothing local changed, the device stays signed
/// out, and the account's two devices still sync.
fn a_refused_sign_in_revokes_its_login(mode: Mode, code: &str) {
    let server = Server::start();
    let (proxy, a, b, email, password) = setup(&server);
    let (live, revoked) = sessions_and_revoked_devices(&server, &email);
    assert_eq!((live, revoked), (2, 0), "A and B");
    let access_before = live_access_tokens(&server, &email);

    let laptop = Client::local("laptop", &new_password());
    laptop.host("laptop.example.com");
    let vault_before = laptop.vault_id();
    proxy.set(mode);
    let refused = laptop
        .core
        .sync_sign_in(shellpp_core::service::SignInInput {
            origin: proxy.origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        });
    match refused {
        Err(CoreError::Sync(error)) => assert_eq!(error.code(), code),
        other => panic!("expected {code}, got {other:?}"),
    }
    proxy.set(Mode::Pass);
    assert_eq!(
        sessions_and_revoked_devices(&server, &email),
        (2, 1),
        "the login's tokens are gone and its device row is revoked"
    );
    assert_eq!(
        live_access_tokens(&server, &email),
        access_before,
        "no access token of the login stays"
    );
    assert!(!laptop.status().signed_in);
    assert_eq!(laptop.vault_id(), vault_before);
    assert_eq!(laptop.addresses(), vec!["laptop.example.com"]);
    a.host("after.example.com");
    a.sync();
    b.sync();
    assert!(b.addresses().contains(&"after.example.com".to_owned()));
}

#[test]
fn a_forged_epoch_at_sign_in_revokes_the_login() {
    a_refused_sign_in_revokes_its_login(Mode::ForgeEpoch, "forged_epoch");
}

#[test]
fn a_newer_epoch_at_sign_in_revokes_the_login() {
    a_refused_sign_in_revokes_its_login(Mode::NewerEpoch, "newer_epoch");
}

#[test]
fn a_key_wrap_that_does_not_open_at_sign_in_revokes_the_login() {
    a_refused_sign_in_revokes_its_login(Mode::TamperLoginWrap, "integrity");
}

/// Not only a refusal (review of run 13): the merge's pull answered `503`
/// fails the sign-in after its login, and that login is revoked too.
#[test]
fn a_server_error_in_the_merge_at_sign_in_revokes_the_login() {
    a_refused_sign_in_revokes_its_login(Mode::FailPull, "unavailable");
}

/// The access tokens of the account (revoking a family deletes its own).
fn live_access_tokens(server: &Server, email: &str) -> i64 {
    server
        .db
        .tx(shellpp_server::db::TxKind::Read, |tx| {
            let rows = tx.query(
                "SELECT COUNT(*) FROM access_tokens t JOIN users u ON u.user_id = t.user_id \
                 WHERE u.email = ?",
                &[shellpp_server::db::Val::from(email)],
                &[shellpp_server::db::Ty::Int],
            )?;
            Ok::<_, shellpp_server::db::DbError>(rows[0][0].int().unwrap_or(0))
        })
        .expect("count")
}

/// Review of run 13: a first run (an empty device, "Restore from sync")
/// whose login's key wrap does not open revokes the device row that login
/// made; signing in again through an honest server works and logs in as
/// the same device, whose row is active again (no row per attempt).
#[test]
fn a_refused_first_run_sign_in_revokes_its_login() {
    let server = Server::start();
    let (proxy, _a, _b, email, password) = setup(&server);
    let c = Client::empty("C");
    let input = || shellpp_core::service::SignInInput {
        origin: proxy.origin.clone(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    };
    proxy.set(Mode::TamperLoginWrap);
    match c.core.sync_sign_in(input()) {
        Err(CoreError::Sync(error)) => assert_eq!(error.code(), "integrity"),
        other => panic!("expected integrity, got {other:?}"),
    }
    proxy.set(Mode::Pass);
    assert_eq!(sessions_and_revoked_devices(&server, &email), (2, 1));
    assert!(matches!(
        c.core.sync_sign_in(input()).expect("signed in"),
        SignInOutcome::Done(_)
    ));
    assert_eq!(sessions_and_revoked_devices(&server, &email), (3, 0));
}

/// Review of run 13 (its second pass): a signed-out device locked while
/// its login is under way fails the rebind with `locked`; the login's
/// session goes and the device's own row stays (a lock keeps the vault id
/// that tells a rebind).
#[test]
fn a_rebind_locked_during_its_login_keeps_the_devices_own_row() {
    let server = Server::start();
    let (proxy, _a, b, email, password) = setup(&server);
    b.core.sync_sign_out(false).expect("signed out");
    proxy.hold_login();
    let result = std::thread::scope(|scope| {
        let signing = scope.spawn(|| {
            b.core.sync_sign_in(shellpp_core::service::SignInInput {
                origin: proxy.origin.clone(),
                email: email.clone(),
                password: password.clone(),
                registration_code: None,
            })
        });
        proxy.wait_login_held();
        b.core.lock().expect("locked");
        proxy.release_login();
        signing.join().expect("the sign-in's thread")
    });
    assert!(matches!(result, Err(CoreError::Locked)), "{result:?}");
    assert_eq!(sessions_and_revoked_devices(&server, &email), (1, 0));
    assert_eq!(live_access_tokens(&server, &email), 1, "A's alone");
}

/// Review of run 13: a device of the account signed out (its refresh
/// token gone, its device row still active) whose sign-in is refused
/// before it is bound again loses that login's session only: its own row
/// stays active (D185 keeps it exempt from the email's soft limit), and
/// it signs in again afterwards.
#[test]
fn a_refused_rebind_keeps_the_devices_own_row() {
    let server = Server::start();
    let (proxy, _a, b, email, password) = setup(&server);
    b.core.sync_sign_out(false).expect("signed out");
    assert_eq!(sessions_and_revoked_devices(&server, &email), (1, 0));
    let input = || shellpp_core::service::SignInInput {
        origin: proxy.origin.clone(),
        email: email.clone(),
        password: password.clone(),
        registration_code: None,
    };
    proxy.set(Mode::TamperLoginWrap);
    match b.core.sync_sign_in(input()) {
        Err(CoreError::Sync(error)) => assert_eq!(error.code(), "integrity"),
        other => panic!("expected integrity, got {other:?}"),
    }
    proxy.set(Mode::Pass);
    assert_eq!(
        sessions_and_revoked_devices(&server, &email),
        (1, 0),
        "the login's refresh token is revoked, B's row is not"
    );
    assert_eq!(live_access_tokens(&server, &email), 1, "A's alone");
    assert!(matches!(
        b.core.sync_sign_in(input()).expect("signed in again"),
        SignInOutcome::Done(_)
    ));
    assert_eq!(sessions_and_revoked_devices(&server, &email), (2, 0));
}
