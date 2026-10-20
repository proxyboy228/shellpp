//! The end of a session (UI 4.6): exit 0 closes, a non-zero exit ends
//! without reconnecting, a transport drop or a keepalive timeout drops and
//! reconnects with backoff (cancellable, off by setting), a failed first
//! attempt is not retried; the connection log and flow control.

use std::net::TcpListener;

use secrecy::ExposeSecret as _;
use shellpp_test_sshd::{KeyType, generate_key};

use super::*;

fn password_user(name: &str, password: &str) -> UserConfig {
    UserConfig::new(name).with_password(secret(password))
}

/// Waits until `count` connection log entries were started.
async fn wait_for_starts(harness: &Harness, count: usize) -> Vec<Uuid> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let starts = harness.calls.starts();
        if starts.len() >= count {
            return starts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{count} connection log starts expected, {} seen",
            starts.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The open connection log entry of the first session.
async fn last_start(harness: &Harness) -> Uuid {
    *wait_for_starts(harness, 1)
        .await
        .last()
        .expect("a connection log start")
}

async fn wait_for_end(harness: &Harness, log_id: Uuid) -> (ConnectionEndReason, Option<u32>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(end) = harness.calls.end_of(log_id) {
            return end;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no connection log end"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_zero_closes_the_session_and_ends_its_log_entry() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target.clone(), true).await;
    let log_id = last_start(&harness).await;
    assert!(matches!(
        harness.calls.log()[0],
        LogCall::Start { username: Some(ref user), host: Some(_), .. } if user == "alice"
    ));
    assert!(
        harness.calls.end_of(log_id).is_none(),
        "open while connected"
    );

    session.type_line("exit");
    let view = session.state(SessionState::Closed).await;
    assert!(view.message().is_none());
    assert_eq!(
        wait_for_end(&harness, log_id).await,
        (ConnectionEndReason::Closed, Some(0))
    );
    assert!(harness.manager.handle(session.handle.id()).is_none());
    assert!(harness.manager.views().is_empty());
    // The event stream ends after the final view.
    assert!(session.events.next().await.is_none());

    // The entry is in the encrypted store (Recent connections reads it).
    let recent = harness.core.recent_connections().expect("recent");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].title, "test host");
    assert_eq!(recent[0].username.as_deref(), Some("alice"));
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_zero_exit_ends_the_session_without_reconnecting() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, true).await;
    let log_id = last_start(&harness).await;
    session.echo_round_trip("before-exit").await;
    let output = session.terminal.len();

    session.type_line("exit 3");
    let view = session.state(SessionState::Ended).await;
    assert_eq!(view.exit_status, Some(3));
    assert!(view.reconnect.is_none());
    assert!(!view.is_active());
    assert_eq!(view.message().as_deref(), Some("Session ended with code 3"));
    assert_eq!(
        wait_for_end(&harness, log_id).await,
        (ConnectionEndReason::ExitStatus, Some(3))
    );
    // Not a drop: nothing happens by itself, the output stays.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    session.drain();
    assert_eq!(session.view.state, SessionState::Ended);
    assert!(session.terminal.len() >= output);
    assert_eq!(harness.calls.starts().len(), 1);

    // Reconnect in the same tab; the terminal modes are reset first.
    let before = session.terminal.len();
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    assert!(session.prompts.iter().all(|p| *p == "host_key_unknown"));
    let after = session.terminal.bytes();
    assert!(
        after[before..]
            .windows(MODE_RESET.len())
            .any(|window| window == MODE_RESET),
        "mode reset before the new shell"
    );
    assert_eq!(wait_for_starts(&harness, 2).await.len(), 2);
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transport_drop_keeps_the_tab_and_reconnects_automatically() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, true).await;
    let first_log = last_start(&harness).await;

    server.drop_connections();
    let view = session
        .until("the countdown", |view| {
            view.state == SessionState::Dropped
                && view.reconnect.is_some_and(|r| r.next_at.is_some())
        })
        .await;
    assert!(matches!(
        view.error,
        Some(SessionError::ConnectionLost | SessionError::ClosedByServer)
    ));
    assert!(view.message().is_some());
    assert!(view.is_active(), "attempts are scheduled");
    let reconnect = view.reconnect.expect("reconnect");
    assert_eq!(reconnect.attempt, 1);
    let left = reconnect
        .seconds_left(std::time::Instant::now())
        .expect("countdown");
    assert!(
        left <= 1,
        "the first attempt comes after 1 second, not {left}"
    );
    assert_eq!(
        wait_for_end(&harness, first_log).await,
        (ConnectionEndReason::Dropped, None)
    );

    session.state(SessionState::Connected).await;
    assert_eq!(wait_for_starts(&harness, 2).await.len(), 2);
    session.echo_round_trip("after-reconnect").await;
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keepalive_timeout_is_a_drop() {
    let harness = Harness::new();
    harness.set("terminal.autoreconnect", json!(false));
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice", "keepalive_s": 1}}),
        Some(&password),
    );
    let mut session = harness.connect(target, true).await;
    server.freeze_connections();
    let view = session.state(SessionState::Dropped).await;
    assert_eq!(view.error, Some(SessionError::KeepaliveTimeout));
    assert_eq!(
        view.message().as_deref(),
        Some(SessionError::KeepaliveTimeout.message())
    );
    // Autoreconnect is off: the session waits for Reconnect.
    assert!(view.reconnect.is_none());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    session.drain();
    assert_eq!(session.view.state, SessionState::Dropped);
    assert_eq!(harness.calls.starts().len(), 1);
    server.drop_connections();
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_automatic_attempts_back_off_and_cancel_stops_them() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, true).await;
    server.set_accepting(false).await.expect("stop listening");
    session.type_line("drop");

    // Attempt 1 after 1 s fails (refused); attempt 2 is 2 s later.
    let first = session
        .until("the first countdown", |view| {
            view.reconnect
                .is_some_and(|r| r.attempt == 1 && r.next_at.is_some())
        })
        .await;
    let first_error = first.error;
    session
        .until("the first attempt", |view| {
            view.reconnect.is_some_and(|r| r.in_progress)
        })
        .await;
    let second = session
        .until("the second countdown", |view| {
            view.reconnect
                .is_some_and(|r| r.attempt == 2 && r.next_at.is_some())
        })
        .await;
    assert_eq!(second.state, SessionState::Dropped);
    assert_eq!(
        second.error,
        Some(SessionError::Ssh(FailureKind::Refused)),
        "the failed attempt's reason is shown"
    );
    let left = second
        .reconnect
        .and_then(|r| r.seconds_left(std::time::Instant::now()))
        .expect("countdown");
    assert!((1..=2).contains(&left), "2 s backoff, {left} left");

    // Cancel stops the automatic attempts; the tab stays dropped.
    session.handle.cancel();
    let view = session
        .until("stopped attempts", |view| view.reconnect.is_none())
        .await;
    assert_eq!(view.state, SessionState::Dropped);
    assert_eq!(view.error, Some(SessionError::Ssh(FailureKind::Refused)));
    tokio::time::sleep(Duration::from_millis(2500)).await;
    session.drain();
    assert!(session.view.reconnect.is_none(), "no more attempts");
    assert!(first_error.is_some());

    // Reconnect by hand once the server is back.
    server.set_accepting(true).await.expect("listen again");
    session.handle.reconnect();
    session.state(SessionState::Connected).await;
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authentication_failures_stop_automatic_attempts() {
    let harness = Harness::new();
    let key = generate_key(KeyType::Ed25519, "test key", None).expect("key");
    let key_file = harness.key_dir.path().join("id_ed25519");
    std::fs::write(&key_file, key.private_openssh.expose_secret()).unwrap();
    let server = start_server(vec![
        UserConfig::new("bob").with_authorized_key(&key.public_openssh),
    ])
    .await;
    let target = harness.host(server.port(), json!({"ssh": {"username": "bob"}}), None);
    let mut session = harness.connect(target, true).await;
    // The key goes away, then the connection drops: the automatic attempt
    // fails authentication, and the attempts stop with that reason.
    std::fs::remove_file(&key_file).unwrap();
    session.type_line("drop");
    let view = session
        .until("stopped attempts", |view| {
            view.error == Some(SessionError::Ssh(FailureKind::AuthFailed))
        })
        .await;
    assert_eq!(view.state, SessionState::Dropped);
    assert!(view.reconnect.is_none());
    tokio::time::sleep(Duration::from_millis(2500)).await;
    session.drain();
    assert!(session.view.reconnect.is_none(), "no more attempts");
    assert_eq!(session.view.state, SessionState::Dropped);
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_prompt_stops_automatic_attempts() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target.clone(), true).await;
    // The saved password becomes wrong, then the connection drops.
    let SessionTarget::Host { record_id, .. } = target else {
        unreachable!()
    };
    harness
        .core
        .save_prompted_password(
            RecordRef {
                vault_id: harness.vault_id,
                record_id,
            },
            secret(&random_secret("other")),
        )
        .expect("password changed");
    session.type_line("drop");
    // The attempt asks again (the saved password was refused); a cancel
    // stops the automatic attempts with the drop reason.
    let (id, _) = session.prompt("password").await;
    session.reply(id, PromptReply::Cancel);
    let view = session
        .until("stopped attempts", |view| {
            view.state == SessionState::Dropped && view.reconnect.is_none()
        })
        .await;
    assert!(view.error.is_some_and(SessionError::is_transport));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    session.drain();
    assert!(session.view.reconnect.is_none());
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_first_attempt_is_not_retried_automatically() {
    let harness = Harness::new();
    // A port nobody listens on.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let target = harness.host(port, json!({"ssh": {"username": "alice"}}), None);
    let mut session = harness.open(target);
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Ssh(FailureKind::Refused)));
    assert_eq!(
        view.message().as_deref(),
        Some("The server refused the connection.")
    );
    assert!(view.reconnect.is_none());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    session.drain();
    assert_eq!(
        session.states,
        vec![SessionState::Connecting, SessionState::Failed]
    );
    assert!(
        harness.calls.starts().is_empty(),
        "no log entry without a session"
    );

    // Retry runs one more attempt.
    session.handle.reconnect();
    session.state(SessionState::Connecting).await;
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Ssh(FailureKind::Refused)));
    session.handle.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_ends_the_log_entry_and_the_server_session() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, true).await;
    let log_id = last_start(&harness).await;
    session.handle.close();
    session.state(SessionState::Closed).await;
    assert_eq!(
        wait_for_end(&harness, log_id).await,
        (ConnectionEndReason::Closed, None)
    );
    let deadline = tokio::time::Instant::now() + WAIT;
    while server.connection_count() > 0 {
        assert!(tokio::time::Instant::now() < deadline, "connection kept");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_waits_for_a_terminal_that_is_behind() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, true).await;
    session.echo_round_trip("ready").await;

    // The terminal stops taking output: the pump waits on one chunk and
    // reads nothing more (the SSH window then stops the server).
    session.terminal.set_open(false);
    let feeds = session.terminal.feeds();
    let held = session.terminal.len();
    session.type_line("lines 5000");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(session.terminal.feeds(), feeds + 1, "one feed waits");
    assert_eq!(session.terminal.len(), held, "nothing passed the terminal");

    // Typing still works while output waits.
    session.type_line("echo typed-while-held");

    // Everything arrives, in order, once the terminal catches up.
    session.terminal.set_open(true);
    session.output_from(held, "line 5000\r\n").await;
    session.output_from(held, "typed-while-held\r\n").await;
    let text = session.terminal.text();
    let first = text.find("line 1\r\n").expect("line 1");
    let last = text.find("line 5000\r\n").expect("line 5000");
    assert!(first < last);
    let count = text.matches("\r\nline ").count();
    assert!(count >= 4999, "{count} lines arrived");

    // A close while the terminal is behind still closes at once.
    session.terminal.set_open(false);
    session.type_line("lines 5000");
    tokio::time::sleep(Duration::from_millis(300)).await;
    session.handle.close();
    session.state(SessionState::Closed).await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_every_session_and_ends_the_log() {
    let mut harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let mut first = harness.connect(target.clone(), true).await;
    let mut second = harness.connect(target, false).await;
    assert_eq!(harness.manager.active_count(), 2);
    let logs = wait_for_starts(&harness, 2).await;
    assert_eq!(logs.len(), 2);
    harness.manager.shutdown();
    first.state(SessionState::Closed).await;
    second.state(SessionState::Closed).await;
    for log_id in logs {
        assert_eq!(
            harness.calls.end_of(log_id),
            Some((ConnectionEndReason::Closed, None))
        );
    }
    assert_eq!(harness.manager.active_count(), 0);
    assert_eq!(
        harness
            .manager
            .open(
                OpenRequest::Local { shell_id: None },
                (80, 24),
                Arc::new(TestTerminal::default())
            )
            .err(),
        Some(SessionError::Locked),
        "nothing opens after shutdown"
    );
    server.shutdown().await;
}
