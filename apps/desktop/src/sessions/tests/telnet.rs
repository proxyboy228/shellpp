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

/// A server that floods option requests and never reads (review, D87): the
/// client stops reading once its replies wait, instead of queueing them
/// without end; closing the tab lets go of the socket all the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_never_reads_cannot_grow_the_queue() {
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
    let mut session = harness
        .open_with(
            OpenRequest::Telnet {
                vault_id,
                record_id,
            },
            Arc::new(TestTerminal::default()),
        )
        .expect("opens");
    let (socket, _) = listener.accept().await.expect("accept");
    let (_read, mut write) = socket.into_split();
    // TERMINAL-TYPE agreed, then SEND again and again: 6 bytes in, 20
    // bytes out each.
    write.write_all(&[IAC, DO, 24]).await.expect("write");
    let flood: Vec<u8> = [IAC, SB, 24, 1, IAC, SE].repeat(64 * 1024);
    let mut sent = 0usize;
    let started = Instant::now();
    while sent < 64 * 1024 * 1024 && started.elapsed() < Duration::from_secs(5) {
        match tokio::time::timeout(Duration::from_millis(500), write.write_all(&flood)).await {
            Ok(Ok(())) => sent += flood.len(),
            Ok(Err(error)) => panic!("the client closed: {error}"),
            Err(_) => break,
        }
    }
    assert!(
        sent < 64 * 1024 * 1024,
        "the client read {sent} bytes of requests it could not answer"
    );
    // Closing the tab ends the stuck writer: the socket closes and the
    // server's writes fail.
    session.handle.close();
    session.state(SessionState::Closed).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let written =
            tokio::time::timeout(Duration::from_millis(200), write.write_all(&flood)).await;
        if matches!(written, Ok(Err(_))) {
            break;
        }
        assert!(Instant::now() < deadline, "the socket stayed open");
    }
}

/// The host's charset applies to Telnet (data-model 7.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telnet_uses_the_host_charset() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
    let port = listener.local_addr().expect("address").port();
    let harness = Harness::new();
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = harness.host(
        22,
        json!({"telnet": {"port": port}, "charset": "windows-1251"}),
        None,
    )
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
    let (mut socket, _) = listener.accept().await.expect("accept");
    // "Привет" in windows-1251.
    socket
        .write_all(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, b' '])
        .await
        .expect("write");
    session.state(SessionState::Connected).await;
    session.output_from(0, "Привет").await;
    // Typed text goes in the same charset.
    session.type_line("да");
    let mut seen = Vec::new();
    read_until(&mut socket, &mut seen, &[0xE4, 0xE0, b'\r', 0]).await;
}
