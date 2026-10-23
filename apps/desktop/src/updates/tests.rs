//! The update check against a server of the test on this machine (no
//! network): newer, not newer, a broken answer, no answer, and no request
//! at all while the check is off (plan of run 6, section 10; D97); the
//! manifest's signature with the release key (D170): known vectors of
//! minisign itself, a round trip with a key of the test, and refusals of
//! changed data.

use std::io::Write as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64ct::{Base64, Encoding as _};
use blake2::{Blake2b512, Digest as _};
use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::{Map, Value, json};

use super::signature::{PublicKey, SignatureError, signature_url};
use super::*;

/// Made by minisign 0.12 with a key whose secret half is gone
/// (testdata/README.md).
const KNOWN_KEY: &str = include_str!("testdata/minisign.pub");
const KNOWN_MANIFEST: &[u8] = include_bytes!("testdata/latest.json");
const KNOWN_SIGNATURE: &str = include_str!("testdata/latest.json.minisig");
const KNOWN_LEGACY: &str = include_str!("testdata/latest.json.legacy.minisig");

/// A minisign key of the test, generated for each run.
struct Signer {
    id: [u8; 8],
    key: SigningKey,
}

impl Signer {
    fn new() -> Self {
        let mut seed = [0u8; 32];
        let mut id = [0u8; 8];
        getrandom::fill(&mut seed).expect("os random");
        getrandom::fill(&mut id).expect("os random");
        Self {
            id,
            key: SigningKey::from_bytes(&seed),
        }
    }

    /// The `.pub` file of the key.
    fn public_file(&self) -> String {
        let mut bytes = Vec::from(*b"Ed");
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(self.key.verifying_key().as_bytes());
        let mut text = [0u8; 64];
        let line = Base64::encode(&bytes, &mut text).unwrap();
        format!("untrusted comment: minisign public key of the test\n{line}\n")
    }

    fn public(&self) -> PublicKey {
        PublicKey::parse(&self.public_file()).expect("a key")
    }

    /// The `.minisig` file of `data`, as minisign makes it (prehashed).
    fn sign(&self, data: &[u8], comment: &str) -> String {
        let hash = Blake2b512::digest(data);
        let file_sig = self.key.sign(&hash).to_bytes();
        let mut sig = Vec::from(*b"ED");
        sig.extend_from_slice(&self.id);
        sig.extend_from_slice(&file_sig);
        let mut signed = file_sig.to_vec();
        signed.extend_from_slice(comment.as_bytes());
        let global = self.key.sign(&signed).to_bytes();
        let (mut a, mut b) = ([0u8; 128], [0u8; 128]);
        format!(
            "untrusted comment: signature of the test\n{}\ntrusted comment: {comment}\n{}\n",
            Base64::encode(&sig, &mut a).unwrap(),
            Base64::encode(&global, &mut b).unwrap()
        )
    }
}

/// A server of the test: counts the requests and keeps the last one's
/// head.
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    head: Arc<std::sync::Mutex<String>>,
}

/// A server that answers every request with `status` and `body`.
fn serve(status: &'static str, body: &'static str) -> Server {
    serve_with(move |_| (status, body.to_owned()))
}

/// A server of `/latest.json` (`manifest`) and its signature (`signature`,
/// none: 404); any other path is 404.
fn serve_signed(manifest: &str, signature: Option<String>) -> Server {
    let manifest = manifest.to_owned();
    serve_with(move |path| match path {
        "/latest.json" => ("200 OK", manifest.clone()),
        "/latest.json.minisig" => match &signature {
            Some(signature) => ("200 OK", signature.clone()),
            None => ("404 Not Found", String::new()),
        },
        _ => ("404 Not Found", String::new()),
    })
}

/// A manifest signed by `signer` with the release workflow's trusted
/// comment, served with its signature.
fn serve_manifest(signer: &Signer, manifest: &str) -> Server {
    serve_signed(manifest, Some(sign_manifest(signer, manifest)))
}

/// The signature the release workflow makes of `manifest`: its trusted
/// comment names the manifest's version ("test" without one).
fn sign_manifest(signer: &Signer, manifest: &str) -> String {
    let version = serde_json::from_str::<Value>(manifest)
        .ok()
        .and_then(|value| value.get("version")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "test".to_owned());
    signer.sign(
        manifest.as_bytes(),
        &format!("shellpp {version} latest.json"),
    )
}

fn serve_with(answer: impl Fn(&str) -> (&'static str, String) + Send + 'static) -> Server {
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
            let text = String::from_utf8_lossy(&request[..read]).into_owned();
            let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
            *kept.lock().unwrap() = text;
            let (status, body) = answer(&path);
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
fn a_signature_of_minisign_itself_verifies() {
    let key = PublicKey::parse(KNOWN_KEY).expect("the key of the vectors");
    assert_eq!(
        key.verify(KNOWN_MANIFEST, KNOWN_SIGNATURE),
        Ok("shellpp 9.9.9 latest.json")
    );
    // CRLF line ends (a file saved on Windows) read the same.
    assert_eq!(
        key.verify(KNOWN_MANIFEST, &KNOWN_SIGNATURE.replace('\n', "\r\n")),
        Ok("shellpp 9.9.9 latest.json")
    );
    // The comment is the release workflow's for this manifest.
    assert!(names_manifest("shellpp 9.9.9 latest.json", KNOWN_MANIFEST));
    // The legacy kind (a signature of the file itself) is refused even
    // though it is the key's.
    assert_eq!(
        key.verify(KNOWN_MANIFEST, KNOWN_LEGACY),
        Err(SignatureError::Malformed)
    );
}

#[test]
fn changed_data_does_not_verify() {
    let key = PublicKey::parse(KNOWN_KEY).unwrap();
    // The manifest: one byte more, one byte changed.
    let mut longer = KNOWN_MANIFEST.to_vec();
    longer.push(b' ');
    assert_eq!(
        key.verify(&longer, KNOWN_SIGNATURE),
        Err(SignatureError::Mismatch)
    );
    let newer = String::from_utf8(KNOWN_MANIFEST.to_vec())
        .unwrap()
        .replace("9.9.9", "9.9.8");
    assert_eq!(
        key.verify(newer.as_bytes(), KNOWN_SIGNATURE),
        Err(SignatureError::Mismatch)
    );
    // The trusted comment is signed too.
    let comment = KNOWN_SIGNATURE.replace("shellpp 9.9.9", "shellpp 9.9.8");
    assert_eq!(
        key.verify(KNOWN_MANIFEST, &comment),
        Err(SignatureError::Mismatch)
    );
    // A flipped bit in the signature or in the global signature.
    let lines: Vec<&str> = KNOWN_SIGNATURE.lines().collect();
    for index in [1, 3] {
        let mut bytes = [0u8; 74];
        let decoded = Base64::decode(lines[index], &mut bytes).unwrap().to_vec();
        let mut flipped = decoded.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        let mut text = [0u8; 128];
        let line = Base64::encode(&flipped, &mut text).unwrap().to_owned();
        let mut changed: Vec<&str> = lines.clone();
        changed[index] = &line;
        assert_eq!(
            key.verify(KNOWN_MANIFEST, &changed.join("\n")),
            Err(SignatureError::Mismatch),
            "line {index}"
        );
    }
    // Not a signature file at all.
    for broken in ["", "not a signature", &lines[..3].join("\n")] {
        assert_eq!(
            key.verify(KNOWN_MANIFEST, broken),
            Err(SignatureError::Malformed),
            "{broken:?}"
        );
    }
    let extra = format!("{KNOWN_SIGNATURE}one more line\n");
    assert_eq!(
        key.verify(KNOWN_MANIFEST, &extra),
        Err(SignatureError::Malformed)
    );
}

#[test]
fn another_key_does_not_verify() {
    let signer = Signer::new();
    let ours = signer.public();
    let data = br#"{"version": "1.0.0"}"#;
    let signature = signer.sign(data, "shellpp 1.0.0 latest.json");
    // The round trip.
    assert_eq!(
        ours.verify(data, &signature),
        Ok("shellpp 1.0.0 latest.json")
    );
    // Another key: its id differs.
    let other = Signer::new();
    assert_eq!(
        other.public().verify(data, &signature),
        Err(SignatureError::OtherKey)
    );
    // Another key under the same id: the signature does not open.
    let impostor = Signer {
        id: signer.id,
        key: Signer::new().key,
    };
    assert_eq!(
        impostor.public().verify(data, &signature),
        Err(SignatureError::Mismatch)
    );
    assert_eq!(
        PublicKey::parse(KNOWN_KEY)
            .unwrap()
            .verify(data, &signature),
        Err(SignatureError::OtherKey)
    );
}

#[test]
fn only_a_minisign_key_is_a_key() {
    assert!(PublicKey::parse(KNOWN_KEY).is_some());
    for text in [
        "",
        "# no key yet\n",
        "untrusted comment: minisign public key\n",
        "untrusted comment: minisign public key\nnot base64\n",
        // A signature line (74 bytes) where the key goes.
        &KNOWN_SIGNATURE
            .lines()
            .take(2)
            .collect::<Vec<_>>()
            .join("\n"),
        // Lines before the comment: minisign itself reads line 2 as the key.
        &format!("# the release key\n{KNOWN_KEY}"),
    ] {
        assert_eq!(PublicKey::parse(text), None, "{text:?}");
    }
    // The release key file holds a key exactly when it starts with the
    // key's comment line: a broken key in it fails here, not in the
    // release.
    let file = include_str!("../../../../release/minisign.pub");
    assert_eq!(
        PublicKey::release().is_some(),
        file.starts_with("untrusted comment:"),
        "release/minisign.pub"
    );
}

#[test]
fn the_signature_sits_next_to_the_manifest() {
    assert_eq!(
        signature_url(DEFAULT_MANIFEST_URL),
        "https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json.minisig"
    );
    assert_eq!(
        signature_url(" https://example.com/m.json?channel=beta "),
        "https://example.com/m.json.minisig?channel=beta"
    );
}

#[test]
fn a_newer_release_is_found_with_its_page() {
    let signer = Signer::new();
    let server = serve_manifest(
        &signer,
        r#"{"version": "v1.4.0", "notes": "", "platforms": {}}"#,
    );
    let outcome = check_with(&server.url, "1.3.9", Some(&signer.public())).unwrap();
    assert_eq!(
        outcome,
        Outcome::Newer {
            version: "1.4.0".to_owned(),
            page: format!("{}/releases/tag/v1.4.0", env!("CARGO_PKG_REPOSITORY")),
        }
    );
    // The manifest and its signature.
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    // No identifiers: our User-Agent and nothing else of ours.
    let head = server.head.lock().unwrap().to_ascii_lowercase();
    assert!(head.contains("user-agent: shellpp/1.3.9"), "{head}");
    assert!(
        !head.contains("cookie") && !head.contains("authorization"),
        "{head}"
    );
}

#[test]
fn an_unsigned_or_changed_manifest_is_not_an_update() {
    let signer = Signer::new();
    let key = signer.public();
    let manifest = r#"{"version": "9.0.0"}"#;
    // No signature next to it.
    let unsigned = serve_signed(manifest, None);
    assert_eq!(
        check_with(&unsigned.url, "0.1.0", Some(&key)),
        Err(CheckError::BadSignature)
    );
    // Signed with another key.
    let other = serve_manifest(&Signer::new(), manifest);
    assert_eq!(
        check_with(&other.url, "0.1.0", Some(&key)),
        Err(CheckError::BadSignature)
    );
    // The signature of another manifest (an older release's).
    let old = signer.sign(br#"{"version": "0.0.9"}"#, "shellpp 0.0.9 latest.json");
    let replayed = serve_signed(manifest, Some(old));
    assert_eq!(
        check_with(&replayed.url, "0.1.0", Some(&key)),
        Err(CheckError::BadSignature)
    );
    // A signature that is not text, or too large to be one.
    let garbage = serve_signed(manifest, Some("x".repeat(5000)));
    assert_eq!(
        check_with(&garbage.url, "0.1.0", Some(&key)),
        Err(CheckError::BadSignature)
    );
    // The key's signature of another file of the release, of the manifest
    // of another version, or with minisign's default comment.
    for comment in [
        "shellpp 9.0.0 shellpp-9.0.0-windows-x64.zip",
        "shellpp 8.0.0 latest.json",
        "timestamp:1 file:latest.json hashed",
    ] {
        let signature = signer.sign(manifest.as_bytes(), comment);
        let server = serve_signed(manifest, Some(signature));
        assert_eq!(
            check_with(&server.url, "0.1.0", Some(&key)),
            Err(CheckError::BadSignature),
            "{comment}"
        );
    }
}

#[test]
fn a_release_published_between_the_two_requests_is_read_again() {
    let signer = Signer::new();
    let old = r#"{"version": "1.0.0"}"#;
    let new = r#"{"version": "1.1.0"}"#;
    let signature = sign_manifest(&signer, new);
    let manifests = Arc::new(AtomicUsize::new(0));
    let server = serve_with(move |path| match path {
        // The first manifest is the old release's; then "latest" moves.
        "/latest.json" if manifests.fetch_add(1, Ordering::SeqCst) == 0 => {
            ("200 OK", old.to_owned())
        }
        "/latest.json" => ("200 OK", new.to_owned()),
        _ => ("200 OK", signature.clone()),
    });
    let outcome = check_with(&server.url, "1.0.0", Some(&signer.public()));
    assert!(
        matches!(&outcome, Ok(Outcome::Newer { version, .. }) if version == "1.1.0"),
        "{outcome:?}"
    );
    // Both asked twice.
    assert_eq!(server.requests.load(Ordering::SeqCst), 4);
}

#[test]
fn a_signature_the_server_cannot_send_now_is_no_answer() {
    let signer = Signer::new();
    for (status, expected) in [
        ("503 Service Unavailable", CheckError::Unreachable),
        ("429 Too Many Requests", CheckError::Unreachable),
        ("404 Not Found", CheckError::BadSignature),
        ("403 Forbidden", CheckError::BadSignature),
    ] {
        let server = serve_with(move |path| match path {
            "/latest.json" => ("200 OK", r#"{"version": "9.0.0"}"#.to_owned()),
            _ => (status, String::new()),
        });
        assert_eq!(
            check_with(&server.url, "0.1.0", Some(&signer.public())),
            Err(expected),
            "{status}"
        );
    }
}

#[test]
fn a_build_without_the_release_key_asks_nothing() {
    let signer = Signer::new();
    let server = serve_manifest(&signer, r#"{"version": "9.0.0"}"#);
    assert_eq!(
        check_with(&server.url, "0.1.0", None),
        Err(CheckError::NoKey)
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    // What the user reads.
    assert_eq!(
        strings::failed(CheckError::NoKey),
        "The update check failed: this build has no key to check updates with"
    );
}

#[test]
fn the_same_or_an_older_release_is_up_to_date() {
    let signer = Signer::new();
    let key = signer.public();
    let server = serve_manifest(&signer, r#"{"version": "0.1.0"}"#);
    assert_eq!(
        check_with(&server.url, "0.1.0", Some(&key)),
        Ok(Outcome::UpToDate)
    );
    assert_eq!(
        check_with(&server.url, "0.2.0", Some(&key)),
        Ok(Outcome::UpToDate)
    );
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
    let signer = Signer::new();
    let key = signer.public();
    // Signed, but not a manifest.
    for body in [
        "not json",
        r#"{"name": "shellpp"}"#,
        r#"{"version": "latest"}"#,
        r#"{"version": 2}"#,
    ] {
        let server = serve_manifest(&signer, body);
        assert_eq!(
            check_with(&server.url, "0.1.0", Some(&key)),
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
    let key = Signer::new().public();
    let server = serve("404 Not Found", "{}");
    assert_eq!(
        check_with(&server.url, "0.1.0", Some(&key)),
        Err(CheckError::Unreachable)
    );
    // A port nothing listens on.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/latest.json",
        closed.local_addr().unwrap().port()
    );
    drop(closed);
    assert_eq!(
        check_with(&url, "0.1.0", Some(&key)),
        Err(CheckError::Unreachable)
    );
}

#[test]
fn only_https_or_this_machine_is_asked() {
    let key = Signer::new().public();
    let key = Some(&key);
    let server = serve("200 OK", r#"{"version": "9.0.0"}"#);
    let plain = server.url.replace("127.0.0.1", "example.com");
    assert_eq!(check_with(&plain, "0.1.0", key), Err(CheckError::NotHttps));
    assert_eq!(
        check_with("ftp://example.com/x", "0.1.0", key),
        Err(CheckError::NotHttps)
    );
    // A user part: not this machine, and no Authorization either.
    let disguised = server.url.replace("127.0.0.1:", "127.0.0.1:@example.com:");
    assert_eq!(
        check_with(&disguised, "0.1.0", key),
        Err(CheckError::NotHttps)
    );
    assert_eq!(
        check_with("https://user:pw@example.com/latest.json", "0.1.0", key),
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

    // Off: not one request; on and due: the manifest and its signature.
    let signer = Signer::new();
    let key = signer.public();
    let server = serve_manifest(&signer, r#"{"version": "0.0.1"}"#);
    let off = values(json!({AUTO_CHECK: false, MANIFEST_URL: server.url.clone()}));
    assert_eq!(at_launch_with(&off, now, Some(&key)), None);
    let recent =
        values(json!({AUTO_CHECK: true, MANIFEST_URL: server.url.clone(), LAST_CHECK: now - 60}));
    assert_eq!(at_launch_with(&recent, now, Some(&key)), None);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    let due = values(json!({AUTO_CHECK: true, MANIFEST_URL: server.url.clone()}));
    assert_eq!(
        at_launch_with(&due, now, Some(&key)),
        Some(Ok(Outcome::UpToDate))
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        manifest_url(&values(json!({MANIFEST_URL: "https://example.com/m.json"}))),
        "https://example.com/m.json"
    );
}

/// xorshift64*: varied inputs, the same for every run (as
/// crates/core/tests/fuzz_parsers.rs; no fuzzing tool is installed).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Pieces of the formats worth inserting.
const TOKENS: &[&str] = &[
    "\n",
    "\r\n",
    " ",
    "=",
    "+",
    "/",
    "untrusted comment: ",
    "trusted comment: ",
    "ED",
    "Ed",
    "RUR",
    "RWR",
    "\"version\": ",
    "\"url\": \"https://",
    "v99999999999999999999.0.0",
    "{",
    "}",
    "\u{0}",
    "\u{202e}",
];

fn mutate(rng: &mut Rng, seed: &str) -> String {
    let mut bytes = seed.as_bytes().to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(bytes.len() + 1);
        match rng.below(5) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] ^= 1 << rng.below(8);
            }
            1 => bytes.insert(at, rng.next() as u8),
            2 if at < bytes.len() => {
                let end = (at + 1 + rng.below(16)).min(bytes.len());
                bytes.drain(at..end);
            }
            3 => {
                let token = TOKENS[rng.below(TOKENS.len())].as_bytes();
                bytes.splice(at..at, token.iter().copied());
            }
            _ => {
                let end = (at + 1 + rng.below(32)).min(bytes.len());
                let chunk = bytes[at.min(end)..end].to_vec();
                bytes.splice(at..at, chunk);
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The signed lines of a signature file (the untrusted comment is not
/// signed, by minisign's design), trimmed as the parser reads them.
fn signed_lines(signature: &str) -> Vec<String> {
    signature
        .lines()
        .skip(1)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Mutated keys, signatures and manifests never panic, and a mutated
/// signature verifies only when its signed lines are unchanged (plan of
/// run 8, C6).
#[test]
fn mutated_keys_signatures_and_manifests_are_refused_without_panic() {
    let key = PublicKey::parse(KNOWN_KEY).unwrap();
    let mut rng = Rng(0x5eed_c620_2610_2300);
    for iteration in 0..500 {
        let signature = mutate(&mut rng, KNOWN_SIGNATURE);
        if key.verify(KNOWN_MANIFEST, &signature).is_ok() {
            assert!(
                signature.starts_with("untrusted comment:")
                    && signed_lines(&signature) == signed_lines(KNOWN_SIGNATURE),
                "iteration {iteration}: a changed signature verified: {signature:?}"
            );
        }
        let manifest = mutate(&mut rng, std::str::from_utf8(KNOWN_MANIFEST).unwrap());
        if key.verify(manifest.as_bytes(), KNOWN_SIGNATURE).is_ok() {
            assert_eq!(
                manifest.as_bytes(),
                KNOWN_MANIFEST,
                "iteration {iteration}: a changed manifest verified"
            );
        }
        let _ = compare(manifest.as_bytes(), "0.1.0");
        if let Some(parsed) = PublicKey::parse(&mutate(&mut rng, KNOWN_KEY)) {
            // Another key never opens the signature of this one.
            if parsed != key {
                assert!(
                    parsed.verify(KNOWN_MANIFEST, KNOWN_SIGNATURE).is_err(),
                    "iteration {iteration}"
                );
            }
        }
    }
}
