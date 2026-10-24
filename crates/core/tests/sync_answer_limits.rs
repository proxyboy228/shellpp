//! A hostile server's answers have a size limit (docs/sync-protocol.md,
//! section 10.2; D188): an answer announced or sent beyond 16 MiB is
//! refused as `answer_too_large` before it is parsed, while one as large as
//! the largest legitimate pull page (under 12 MiB) is read. A push answer
//! may carry one envelope per item, so its limit grows with the batch.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use shellpp_core::sync::http::{MAX_ANSWER_BYTES, push_answer_limit};
use shellpp_core::sync::{HttpApi, HttpError, TrustMode};

#[derive(Clone, Copy)]
enum Answer {
    /// `content-length` beyond the limit (a few bytes sent).
    Announced,
    /// A chunked body that never ends before 17 MiB.
    Streamed,
    /// A valid server info with 12 MiB of an unknown field.
    Large,
}

const INFO_HEAD: &str = r#"{"version":"0.7.0","api_versions":["v1"],"registration":"open","smtp":false,"limits":{"max_envelope_bytes":1,"max_push_items":1,"max_push_body_bytes":1,"default_pull_limit":1,"max_pull_limit":1,"tombstone_retention_days":90},"server_generation":null,"padding":""#;

/// A plain-HTTP server on 127.0.0.1 (allowed for loopback) answering every
/// request with `answer`.
fn serve(answer: Answer) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let mib = vec![b'a'; 1024 * 1024];
                match answer {
                    Answer::Announced => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 17000000\r\n\r\n{\"version\":",
                        );
                    }
                    Answer::Streamed => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n",
                        );
                        for _ in 0..17 {
                            let head = format!("{:x}\r\n", mib.len());
                            if stream.write_all(head.as_bytes()).is_err()
                                || stream.write_all(&mib).is_err()
                                || stream.write_all(b"\r\n").is_err()
                            {
                                return;
                            }
                        }
                        let _ = stream.write_all(b"0\r\n\r\n");
                    }
                    Answer::Large => {
                        let tail = b"\"}";
                        let length = INFO_HEAD.len() + 12 * mib.len() + tail.len();
                        let head = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(INFO_HEAD.as_bytes());
                        for _ in 0..12 {
                            if stream.write_all(&mib).is_err() {
                                return;
                            }
                        }
                        let _ = stream.write_all(tail);
                    }
                }
                let _ = stream.flush();
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn info(origin: &str) -> Result<(), HttpError> {
    HttpApi::new(origin, TrustMode::System, "0.7.0")
        .expect("api")
        .server_info()
        .map(|_| ())
}

#[test]
fn an_announced_answer_over_the_limit_is_refused_unread() {
    let origin = serve(Answer::Announced);
    assert_eq!(info(&origin), Err(HttpError::Refused("answer_too_large")));
}

#[test]
fn a_streamed_answer_over_the_limit_is_refused() {
    let origin = serve(Answer::Streamed);
    assert_eq!(info(&origin), Err(HttpError::Refused("answer_too_large")));
}

#[test]
fn an_answer_as_large_as_a_full_pull_page_is_read() {
    let origin = serve(Answer::Large);
    assert_eq!(info(&origin), Ok(()));
}

/// The largest legitimate pull page fits: 8 MiB of envelopes plus one more
/// envelope, base64url, with room for the JSON of 1,000 records.
#[test]
fn the_limits_cover_the_largest_legitimate_answers() {
    let envelopes: usize = 8 * 1024 * 1024 + 262_232;
    let base64 = envelopes.div_ceil(3) * 4;
    assert!(base64 + 1_000 * 64 < MAX_ANSWER_BYTES);
    let one = 262_232usize.div_ceil(3) * 4;
    assert!(push_answer_limit(500) >= MAX_ANSWER_BYTES + 500 * (one + 128));
    assert_eq!(push_answer_limit(0), MAX_ANSWER_BYTES);
}

/// A push answer may be larger than 16 MiB (a conflict per item carries
/// the server's envelope): `push` reads up to its own limit, so 20 MiB for
/// 100 items is read, while the same answer to any other request is not.
#[test]
fn a_push_answer_has_the_limit_of_its_batch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                // The whole request first (head and body).
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 64 * 1024];
                let length = loop {
                    if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break end + 4 + length;
                    }
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                };
                while buffer.len() < length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = br#"{"results":[],"padding":""#;
                let tail = b"\"}";
                let padding = 20 * 1024 * 1024;
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    head.len() + padding + tail.len()
                );
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.write_all(head);
                let _ = stream.write_all(&vec![b'a'; padding]);
                let _ = stream.write_all(tail);
                let _ = stream.flush();
            });
        }
    });
    let origin = format!("http://127.0.0.1:{port}");
    let api = HttpApi::new(&origin, TrustMode::System, "0.7.0").expect("api");
    let items: Vec<shellpp_sync_api::api::RecordWrite> = (0..100)
        .map(|_| shellpp_sync_api::api::RecordWrite {
            base_revision: 0,
            envelope: shellpp_sync_api::api::B64(vec![0u8; 344]),
        })
        .collect();
    let answer = api
        .push("shellpp_at_x", uuid::Uuid::nil(), items)
        .expect("a push answer within its batch's limit");
    assert!(answer.results.is_empty());
    assert_eq!(
        api.server_info().map(|_| ()),
        Err(HttpError::Refused("answer_too_large")),
        "16 MiB for any other answer"
    );
}
