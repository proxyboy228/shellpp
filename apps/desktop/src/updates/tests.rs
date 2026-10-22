//! The update check against a server of the test on this machine (no
//! network): newer, not newer, a broken answer, no answer, and no request
//! at all while the check is off (plan of run 6, section 10; D97).

use std::io::Write as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::*;

/// A server that answers every request with `status` and `body`; counts
/// the requests and keeps the last one's head.
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    head: Arc<std::sync::Mutex<String>>,
}

fn serve(status: &'static str, body: &'static str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/latest.json",
        listener.local_addr().unwrap().port()
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let head = Arc::new(std::sync::Mutex::new(String::new()));
    let (counted, kept) = (requests.clone(), head.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            *kept.lock().unwrap() = String::from_utf8_lossy(&request[..read]).into_owned();
            let answer = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    Server {
        url,
        requests,
        head,
    }
}

#[test]
fn a_newer_release_is_found_with_its_page() {
    let server = serve(
        "200 OK",
        r#"{"version": "v1.4.0", "notes": "", "platforms": {}}"#,
    );
    let outcome = check(&server.url, "1.3.9").unwrap();
    assert_eq!(
        outcome,
        Outcome::Newer {
            version: "1.4.0".to_owned(),
            page: format!("{}/releases/tag/v1.4.0", env!("CARGO_PKG_REPOSITORY")),
        }
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    // No identifiers: our User-Agent and nothing else of ours.
    let head = server.head.lock().unwrap().to_ascii_lowercase();
    assert!(head.contains("user-agent: shellpp/1.3.9"), "{head}");
    assert!(
        !head.contains("cookie") && !head.contains("authorization"),
        "{head}"
    );
}

#[test]
fn the_same_or_an_older_release_is_up_to_date() {
    let server = serve("200 OK", r#"{"version": "0.1.0"}"#);
    assert_eq!(check(&server.url, "0.1.0"), Ok(Outcome::UpToDate));
    assert_eq!(check(&server.url, "0.2.0"), Ok(Outcome::UpToDate));
    // Build metadata does not make a version newer.
    assert_eq!(
        compare(br#"{"version": "0.2.0+ci.5"}"#, "0.2.0"),
        Ok(Outcome::UpToDate)
    );
    // A pre-release of the same version is older than the release.
    assert_eq!(
        compare(br#"{"version": "0.2.0-rc.1"}"#, "0.2.0"),
        Ok(Outcome::UpToDate)
    );
    assert!(matches!(
        compare(br#"{"version": "0.2.0"}"#, "0.2.0-rc.1"),
        Ok(Outcome::Newer { .. })
    ));
}

#[test]
fn a_broken_answer_is_not_a_manifest() {
    for body in [
        "not json",
        r#"{"name": "shellpp"}"#,
        r#"{"version": "latest"}"#,
        r#"{"version": 2}"#,
    ] {
        let server = serve("200 OK", body);
        assert_eq!(
            check(&server.url, "0.1.0"),
            Err(CheckError::BadManifest),
            "{body}"
        );
    }
    // A manifest's page is used only when it is https.
    let outcome = compare(
        br#"{"version": "9.0.0", "url": "http://example.com/x"}"#,
        "0.1.0",
    );
    assert!(
        matches!(&outcome, Ok(Outcome::Newer { page, .. }) if page.starts_with("https://")),
        "{outcome:?}"
    );
}

#[test]
fn no_answer_is_unreachable() {
    let server = serve("404 Not Found", "{}");
    assert_eq!(check(&server.url, "0.1.0"), Err(CheckError::Unreachable));
    // A port nothing listens on.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/latest.json",
        closed.local_addr().unwrap().port()
    );
    drop(closed);
    assert_eq!(check(&url, "0.1.0"), Err(CheckError::Unreachable));
}

#[test]
fn only_https_or_this_machine_is_asked() {
    let server = serve("200 OK", r#"{"version": "9.0.0"}"#);
    let plain = server.url.replace("127.0.0.1", "example.com");
    assert_eq!(check(&plain, "0.1.0"), Err(CheckError::NotHttps));
    assert_eq!(
        check("ftp://example.com/x", "0.1.0"),
        Err(CheckError::NotHttps)
    );
    // A user part: not this machine, and no Authorization either.
    let disguised = server.url.replace("127.0.0.1:", "127.0.0.1:@example.com:");
    assert_eq!(check(&disguised, "0.1.0"), Err(CheckError::NotHttps));
    assert_eq!(
        check("https://user:pw@example.com/latest.json", "0.1.0"),
        Err(CheckError::NotHttps)
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    // The scheme's case does not matter.
    assert_eq!(allowed("HTTPS://example.com/x"), Ok(true));
}

#[test]
fn the_check_at_launch_is_off_by_default_and_daily() {
    let day = CHECK_INTERVAL.as_secs();
    let now = 100 * day;
    let values = |pairs: Value| -> Map<String, Value> { pairs.as_object().cloned().unwrap() };
    assert!(!due_at_launch(&values(json!({})), now), "off by default");
    assert!(!due_at_launch(
        &values(json!({AUTO_CHECK: false, LAST_CHECK: 0})),
        now
    ));
    assert!(due_at_launch(&values(json!({AUTO_CHECK: true})), now));
    assert!(!due_at_launch(
        &values(json!({AUTO_CHECK: true, LAST_CHECK: now - day + 60})),
        now
    ));
    assert!(due_at_launch(
        &values(json!({AUTO_CHECK: true, LAST_CHECK: now - day})),
        now
    ));
    // A last check in the future (the clock went back) does not block it.
    assert!(due_at_launch(
        &values(json!({AUTO_CHECK: true, LAST_CHECK: now + day})),
        now
    ));
    assert_eq!(manifest_url(&values(json!({}))), DEFAULT_MANIFEST_URL);

    // Off: not one request; on and due: one.
    let server = serve("200 OK", r#"{"version": "0.0.1"}"#);
    let off = values(json!({AUTO_CHECK: false, MANIFEST_URL: server.url.clone()}));
    assert_eq!(at_launch(&off, now), None);
    let recent =
        values(json!({AUTO_CHECK: true, MANIFEST_URL: server.url.clone(), LAST_CHECK: now - 60}));
    assert_eq!(at_launch(&recent, now), None);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    let due = values(json!({AUTO_CHECK: true, MANIFEST_URL: server.url.clone()}));
    assert_eq!(at_launch(&due, now), Some(Ok(Outcome::UpToDate)));
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        manifest_url(&values(json!({MANIFEST_URL: "https://example.com/m.json"}))),
        "https://example.com/m.json"
    );
}
