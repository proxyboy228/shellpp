//! Deterministic fuzzing (plan run 7, section 4; as in run 3): mutated
//! request bodies and query strings against the server's routes, and
//! mutated answers against the client's parsers. A fixed seed gives the
//! same inputs every run; a failure names its iteration.
//!
//! The server must answer every request with a status below 500 (no panic,
//! no internal error from input) and keep serving; the client's parsing
//! must refuse or accept without panicking.

mod support;

use std::io::{Read, Write};
use std::net::TcpStream;

use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};
use shellpp_sync_api::api as wire;
use support::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const TOKENS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    ",",
    ":",
    "\"",
    "\\",
    "null",
    "true",
    "-1",
    "0",
    "1e309",
    "18446744073709551616",
    "\"\"",
    "\u{0}",
    "\u{202e}",
    "AAAA",
    "=",
    "%00",
    "&",
    "?",
];

/// One mutation of `input`: flip, delete, insert a token, duplicate or cut.
fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(out.len().max(1));
        match rng.below(5) {
            0 if !out.is_empty() => {
                let i = at.min(out.len() - 1);
                out[i] ^= 1 << rng.below(8);
            }
            1 if !out.is_empty() => {
                let end = (at + 1 + rng.below(8)).min(out.len());
                out.drain(at.min(end)..end);
            }
            2 => {
                let token = TOKENS[rng.below(TOKENS.len())].as_bytes();
                let at = at.min(out.len());
                out.splice(at..at, token.iter().copied());
            }
            3 if !out.is_empty() => {
                let end = (at + 1 + rng.below(32)).min(out.len());
                let piece = out[at.min(end)..end].to_vec();
                let at = at.min(out.len());
                out.splice(at..at, piece);
            }
            _ => out.truncate(at),
        }
    }
    out
}

fn b64(len: usize, byte: u8) -> String {
    BASE64URL_NOPAD.encode(&vec![byte; len])
}

/// A raw HTTP/1.1 exchange; the status code (0 when no answer).
fn http(origin: &str, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> u16 {
    let addr = origin.trim_start_matches("http://");
    let Ok(mut stream) = TcpStream::connect(addr) else {
        return 0;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\n{auth}content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).is_err() || stream.write_all(body).is_err() {
        return 0;
    }
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let text = String::from_utf8_lossy(&answer);
    text.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// A registration body the server accepts (opaque wraps of the right size).
fn register_body(email: &str) -> Value {
    json!({
        "email": email,
        "kdf": {"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4},
        "kdf_salt": b64(16, 1),
        "auth_key": b64(32, 2),
        "recovery_auth_key": b64(32, 3),
        "personal_vault": {
            "vault_id": uuid::Uuid::now_v7(),
            "epoch": 1,
            "kek_wrap": b64(73, 4),
            "recovery_wrap": b64(73, 5),
            "epoch_chain": []
        },
        "x25519_public_key": b64(32, 6),
        "ssh_id_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0 shellpp:x",
        "device_id": uuid::Uuid::now_v7()
    })
}

#[test]
fn mutated_requests_never_break_the_server() {
    let server = Server::start();
    let origin = server.origin.clone();
    // An account for the authenticated routes.
    let email = email();
    let body = register_body(&email);
    let vault_id = body["personal_vault"]["vault_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let status = http(
        &origin,
        "POST",
        "/v1/auth/register",
        None,
        body.to_string().as_bytes(),
    );
    assert_eq!(status, 201, "the seed registration");
    let login = json!({"email": email, "auth_key": b64(32, 2), "device_id": body["device_id"]});
    let token = {
        let addr = origin.trim_start_matches("http://");
        let mut stream = TcpStream::connect(addr).expect("connect");
        let payload = login.to_string();
        let head = format!(
            "POST /v1/auth/login HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            payload.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(payload.as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        let json = &answer[answer.find("\r\n\r\n").unwrap() + 4..];
        let value: Value = serde_json::from_str(json).expect("login answer");
        value["tokens"]["access_token"]
            .as_str()
            .expect("token")
            .to_owned()
    };
    let changes = format!("/v1/vaults/{vault_id}/changes");
    let push = json!({"items": [{"base_revision": 0, "envelope": b64(300, 7)}]});
    let seeds: Vec<(&str, String, Option<&str>, Vec<u8>)> = vec![
        ("POST", "/v1/auth/prelogin".into(), None, json!({"email": "a@b.c"}).to_string().into_bytes()),
        ("POST", "/v1/auth/register".into(), None, register_body("fuzz@example.com").to_string().into_bytes()),
        ("POST", "/v1/auth/login".into(), None, login.to_string().into_bytes()),
        ("POST", "/v1/auth/login/totp".into(), None, json!({"login_ticket": "shellpp_lt_x", "code": "123456"}).to_string().into_bytes()),
        ("POST", "/v1/auth/token/refresh".into(), None, json!({"refresh_token": "shellpp_rt_x"}).to_string().into_bytes()),
        ("POST", "/v1/auth/recover/begin".into(), None, json!({"email": email, "recovery_auth_key": b64(32, 3), "device_id": body["device_id"]}).to_string().into_bytes()),
        ("POST", changes.clone(), Some(token.as_str()), push.to_string().into_bytes()),
        ("POST", "/v1/account/password".into(), Some(token.as_str()), json!({"current_auth_key": b64(32, 2), "kdf": body["kdf"], "kdf_salt": b64(16, 9), "auth_key": b64(32, 2), "kek_wrap": b64(73, 4)}).to_string().into_bytes()),
        ("POST", format!("/v1/vaults/{vault_id}/key-epochs"), Some(token.as_str()), json!({"current_auth_key": b64(32, 2), "epoch": 2, "prev_key_wrap": b64(73, 1), "account_wraps": {"kek_wrap": b64(73, 1), "recovery_wrap": b64(73, 1)}}).to_string().into_bytes()),
    ];
    let mut rng = Rng(0x00f0_220f_u64);
    for iteration in 0..600 {
        let (method, path, token, seed) = &seeds[iteration % seeds.len()];
        let body = mutate(&mut rng, seed);
        let status = http(&origin, method, path, *token, &body);
        assert!(
            status != 0 && status < 500,
            "iteration {iteration}: {method} {path} answered {status}"
        );
    }
    // Query strings of the pull.
    for iteration in 0..200 {
        let query = String::from_utf8_lossy(&mutate(&mut rng, b"since=0&limit=100&classes=1,2,3"))
            .replace([' ', '\r', '\n', '#'], "");
        let path = format!("{changes}?{query}");
        let status = http(&origin, "GET", &path, Some(&token), b"");
        assert!(
            status != 0 && status < 500,
            "pull iteration {iteration}: {path} answered {status}"
        );
    }
    assert_eq!(
        http(&origin, "GET", "/v1/server", None, b""),
        200,
        "still serving"
    );
}

#[test]
fn mutated_answers_never_panic_the_client_parsers() {
    let page = json!({
        "records": [{"seq": 1, "envelope": b64(300, 1)}, {"seq": 2, "envelope": b64(200, 2)}],
        "next_cursor": 2, "has_more": false, "current_epoch": 1, "min_valid_seq": 0
    });
    let push = json!({"results": [
        {"record_id": uuid::Uuid::now_v7(), "status": "ok", "seq": 3},
        {"record_id": uuid::Uuid::now_v7(), "status": "conflict", "current": {"seq": 4, "envelope": b64(80, 3)}},
        {"record_id": uuid::Uuid::now_v7(), "status": "conflict", "current": null}
    ]});
    let info = json!({"version": "0.7.0", "api_versions": ["v1"], "registration": "invite", "smtp": false,
        "limits": {"max_envelope_bytes": 1, "max_push_items": 1, "max_push_body_bytes": 1, "default_pull_limit": 1, "max_pull_limit": 1, "tombstone_retention_days": 90},
        "server_generation": b64(16, 4)});
    let error = json!({"error": {"code": "cursor_expired", "message": "x", "details": {"reason": "purged"}}});
    let mut rng = Rng(0x0c11_e117_u64);
    for iteration in 0..3000 {
        let seed = match iteration % 4 {
            0 => page.to_string(),
            1 => push.to_string(),
            2 => info.to_string(),
            _ => error.to_string(),
        };
        let bytes = mutate(&mut rng, seed.as_bytes());
        let _ = serde_json::from_slice::<wire::PullResponse>(&bytes);
        let _ = serde_json::from_slice::<wire::PushResponse>(&bytes);
        let _ = serde_json::from_slice::<wire::ServerInfo>(&bytes);
        let _ = serde_json::from_slice::<wire::ErrorBody>(&bytes);
        let _ = serde_json::from_slice::<wire::LoginResponse>(&bytes);
        let _ = serde_json::from_slice::<wire::AccountBundle>(&bytes);
        // Envelope headers of the mutated bytes.
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && let Some(text) = value.pointer("/records/0/envelope").and_then(Value::as_str)
            && let Ok(raw) = BASE64URL_NOPAD.decode(text.as_bytes())
        {
            let _ = shellpp_sync_api::envelope::split(&raw);
        }
    }
}
