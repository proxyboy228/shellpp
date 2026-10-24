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
