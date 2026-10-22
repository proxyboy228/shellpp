//! Telnet sessions (D87) against a small Telnet server of the test: the
//! options (ECHO and SUPPRESS-GO-AHEAD from the server, TERMINAL-TYPE and
//! NAWS from the client), the window size again on resize, input and output
//! through the protocol, a drop when the server closes, and Reconnect.

use std::time::Instant;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::*;

const IAC: u8 = 255;
const WILL: u8 = 251;
const DO: u8 = 253;
const SB: u8 = 250;
const SE: u8 = 240;

/// Reads from `socket` into `seen` until it holds `wanted`.
async fn read_until(socket: &mut TcpStream, seen: &mut Vec<u8>, wanted: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !seen.windows(wanted.len()).any(|window| window == wanted) {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut buffer = [0u8; 512];
        let read = tokio::time::timeout(left, socket.read(&mut buffer))
            .await
            .unwrap_or_else(|_| panic!("the client did not send {wanted:?}; it sent {seen:?}"))
            .expect("read");
        assert!(read > 0, "the client closed");
        seen.extend_from_slice(&buffer[..read]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_telnet_session_negotiates_runs_and_reconnects() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let port = listener.local_addr().expect("address").port();
    let harness = Harness::new();
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = harness.host(22, json!({"telnet": {"port": port}}), None)
    else {
        panic!("a saved host");
    };
    let terminal = Arc::new(TestTerminal::default());
    let mut session = harness
        .open_with(
            OpenRequest::Telnet {
                vault_id,
                record_id,
            },
            terminal,
        )
        .expect("opens");
    assert_eq!(session.view.kind, SessionKind::Telnet);
    assert_eq!(session.view.port, Some(port));

    let (mut socket, _) = listener.accept().await.expect("accept");
    // The server asks for the terminal type and the window size, and
    // offers to echo and to suppress go-ahead.
    socket
        .write_all(&[IAC, DO, 24, IAC, DO, 31, IAC, WILL, 1, IAC, WILL, 3])
        .await
        .expect("write");
    let mut seen = Vec::new();
    for wanted in [
        &[IAC, 251, 24][..],
        &[IAC, 251, 31],
        &[IAC, SB, 31],
        &[IAC, DO, 1],
        &[IAC, DO, 3],
    ] {
        read_until(&mut socket, &mut seen, wanted).await;
    }
    socket
        .write_all(&[IAC, SB, 24, 1, IAC, SE])
        .await
        .expect("write");
    let mut is = vec![IAC, SB, 24, 0];
    is.extend_from_slice(b"XTERM-256COLOR");
    is.extend_from_slice(&[IAC, SE]);
    read_until(&mut socket, &mut seen, &is).await;

    // Output through the protocol; a doubled 0xFF and CR NUL are data.
    socket
        .write_all(b"\r\0Welcome\r\nlogin: ")
        .await
        .expect("write");
    session.state(SessionState::Connected).await;
    session.output_from(0, "login: ").await;
    assert!(!session.terminal.text().contains('\0'));

    // Input: Enter goes as CR NUL.
    session.type_line("dev");
    read_until(&mut socket, &mut seen, b"dev\r\0").await;
    // A new window size goes as NAWS.
    session.handle.resize(100, 30);
    read_until(
        &mut socket,
        &mut seen,
        &[IAC, SB, 31, 0, 100, 0, 30, IAC, SE],
    )
    .await;

    // The server closes: a drop, the output kept.
    drop(socket);
    let view = session.state(SessionState::Dropped).await;
    assert_eq!(view.error, Some(SessionError::ClosedByServer));
    assert!(session.terminal.text().contains("login: "));

    // Reconnect connects again.
    session.handle.reconnect();
    let (mut socket, _) = listener.accept().await.expect("accept again");
    socket.write_all(b"login: ").await.expect("write");
    session.state(SessionState::Connected).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_telnet_port_fails_and_a_host_without_telnet_is_refused() {
    // A port nothing listens on.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
        listener.local_addr().expect("address").port()
    };
    let harness = Harness::new();
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = harness.host(22, json!({"telnet": {"port": port}}), None)
    else {
        panic!("a saved host");
    };
    let mut session = harness
        .open_with(
            OpenRequest::Telnet {
                vault_id,
                record_id,
            },
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Ssh(FailureKind::Refused)));

    let SessionTarget::Host {
        vault_id,
        record_id,
    } = harness.host(22, json!({}), None)
    else {
        panic!("a saved host");
    };
    let refused = harness.open_with(
        OpenRequest::Telnet {
            vault_id,
            record_id,
        },
        Arc::new(TestTerminal::default()),
    );
    assert!(matches!(refused, Err(SessionError::InvalidTarget)));
}
