//! Shell and exec channel tests against the in-process russh test server:
//! PTY size and resize, TERM, env requests, exit statuses (D18: an exit
//! status is never a drop), drops, keepalive requests and the keepalive
//! timeout on a frozen link, exec output, limits and timeouts, several
//! channels on one connection.

mod support;

use std::time::{Duration, Instant};

use secrecy::SecretString;
use shellpp_ssh::{Connection, FailureKind, SessionEnd, ShellRequest, connect};
use shellpp_test_sshd::{ExecAnswer, ServerConfig, ServerEvent, TestServer};
use support::net::FreezeProxy;
use support::*;

const PROMPT: &str = "alice@shellpp-test:~$ ";

async fn server_and_connection(config: ServerConfig, password: &str) -> (TestServer, Connection) {
    let server = start_with(config).await;
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    (server, connection)
}

async fn simple() -> (TestServer, Connection) {
    let password = random_secret("pw");
    server_and_connection(
        ServerConfig::new(vec![password_user("alice", &password)]),
        &password,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pty_has_the_requested_size_and_follows_resizes() {
    let (server, connection) = simple().await;
    let request = ShellRequest {
        cols: 100,
        rows: 40,
        ..shell_request()
    };
    let mut shell = connection.open_shell(request).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"size\r").await.expect("write");
    read_until(&mut shell, "\r\n100x40\r\n", SHORT).await;
    shell.writer().resize(120, 50).await.expect("resize");
    shell.writer().write(b"size\r").await.expect("write");
    read_until(&mut shell, "\r\n120x50\r\n", SHORT).await;
    let events = server.events();
    assert!(events.contains(&ServerEvent::Pty {
        term: "xterm-256color".to_owned(),
        cols: 100,
        rows: 40
    }));
    assert!(events.contains(&ServerEvent::WindowChange {
        cols: 120,
        rows: 50
    }));
    assert!(events.contains(&ServerEvent::Shell));
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn term_and_environment_reach_the_server() {
    let (server, connection) = simple().await;
    let request = ShellRequest {
        term: "vt220".to_owned(),
        env: vec![
            ("LANG".to_owned(), SecretString::from("C.UTF-8".to_owned())),
            (
                "SHELLPP_TEST".to_owned(),
                SecretString::from("value-1".to_owned()),
            ),
        ],
        ..shell_request()
    };
    let mut shell = connection.open_shell(request).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"term\r").await.expect("write");
    read_until(&mut shell, "\r\nvt220\r\n", SHORT).await;
    shell
        .writer()
        .write(b"env SHELLPP_TEST\r")
        .await
        .expect("write");
    read_until(&mut shell, "\r\nvalue-1\r\n", SHORT).await;
    let events = server.events();
    assert!(events.contains(&ServerEvent::Env {
        name: "LANG".to_owned()
    }));
    assert!(events.contains(&ServerEvent::Env {
        name: "SHELLPP_TEST".to_owned()
    }));
    connection.disconnect().await;
    server.shutdown().await;
}

/// A server that accepts only some names (OpenSSH `AcceptEnv`): the shell
/// opens anyway and the refused names are reported (stage 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_environment_variables_are_reported() {
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.accept_env = Some(vec!["LANG".to_owned(), "LC_*".to_owned()]);
    let server = start_with(config).await;
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    let value = |text: &str| SecretString::from(text.to_owned());
    let request = ShellRequest {
        env: vec![
            ("LANG".to_owned(), value("C.UTF-8")),
            ("APP_TOKEN".to_owned(), value("t-1")),
            ("LC_ALL".to_owned(), value("C")),
            ("DEPLOY_ENV".to_owned(), value("prod")),
        ],
        ..shell_request()
    };
    let mut shell = connection.open_shell(request).await.expect("shell");
    assert_eq!(shell.refused_env(), ["APP_TOKEN", "DEPLOY_ENV"]);
    read_until(&mut shell, PROMPT, SHORT).await;
    // Nothing refused: the list is empty.
    let shell = connection.open_shell(shell_request()).await.expect("shell");
    assert!(shell.refused_env().is_empty());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_statuses_end_the_shell() {
    let (server, connection) = simple().await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"exit 3\r").await.expect("write");
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(output.end, Some(SessionEnd::ExitStatus(3)), "{output:?}");
    // After the end the shell keeps reporting it.
    assert_eq!(
        shell.next().await,
        shellpp_ssh::ShellEvent::End(SessionEnd::ExitStatus(3))
    );
    // The connection stays usable for another shell.
    assert!(!connection.is_closed());
    let mut second = connection
        .open_shell(shell_request())
        .await
        .expect("second shell");
    read_until(&mut second, PROMPT, SHORT).await;
    second.writer().write(b"exit\r").await.expect("write");
    let output = read_to_end(&mut second, SHORT).await;
    assert_eq!(output.end, Some(SessionEnd::ExitStatus(0)), "{output:?}");
    assert!(server.events().contains(&ServerEvent::Closed {
        exit_status: Some(3)
    }));
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ctrl_d_on_an_empty_line_exits_with_zero() {
    let (server, connection) = simple().await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(&[0x04]).await.expect("write");
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(output.end, Some(SessionEnd::ExitStatus(0)));
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tcp_close_without_exit_status_is_a_drop() {
    let (server, connection) = simple().await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"drop\r").await.expect("write");
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(
        output.end,
        Some(SessionEnd::Dropped(FailureKind::Disconnected)),
        "{output:?}"
    );
    assert!(eventually(SHORT, || connection.is_closed()).await);
    let error = shell.writer().write(b"x").await.expect_err("closed");
    assert_eq!(error.kind, FailureKind::Disconnected);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_connections_end_every_shell_as_dropped() {
    let (server, connection) = simple().await;
    let mut first = connection.open_shell(shell_request()).await.expect("shell");
    let mut second = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut first, PROMPT, SHORT).await;
    read_until(&mut second, PROMPT, SHORT).await;
    assert!(server.drop_connections() >= 1);
    for shell in [&mut first, &mut second] {
        let output = read_to_end(shell, SHORT).await;
        assert_eq!(
            output.end,
            Some(SessionEnd::Dropped(FailureKind::Disconnected)),
            "{output:?}"
        );
    }
    assert!(eventually(SHORT, || connection.is_closed()).await);
    server.shutdown().await;
}

/// An idle connection through a relay that counts client-to-server bytes
/// (the russh test server answers keepalives without telling its handler,
/// so they are observed on the wire).
async fn idle_upstream_bytes(keepalive: Option<Duration>) -> (u64, bool) {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let proxy = FreezeProxy::start(server.addr()).await;
    let mut spec = spec(proxy.addr(), Some("alice"), password_plan(&password));
    spec.keepalive = keepalive;
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    let before = proxy.upstream_bytes();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let sent = proxy.upstream_bytes() - before;
    // Answered keepalives keep the connection.
    let alive = !connection.is_closed();
    shell
        .writer()
        .write(b"echo still-here\r")
        .await
        .expect("write");
    read_until(&mut shell, "\r\nstill-here\r\n", SHORT).await;
    connection.disconnect().await;
    server.shutdown().await;
    (sent, alive)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keepalive_requests_are_sent_and_answered() {
    let (sent, alive) = idle_upstream_bytes(Some(Duration::from_millis(200))).await;
    // Several encrypted global requests in 1.5 s (each packet is at least
    // a few dozen bytes).
    assert!(sent >= 100, "keepalive traffic: {sent} bytes");
    assert!(alive);
}

/// One keepalive per interval (not one per two intervals).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keepalives_go_out_once_per_interval() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let mut spec = spec(server.addr(), Some("alice"), password_plan(&password));
    spec.keepalive = Some(Duration::from_millis(250));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    let count = || {
        server
            .events()
            .into_iter()
            .filter(|event| matches!(event, ServerEvent::Keepalive))
            .count()
    };
    let before = count();
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let sent = count() - before;
    // About eight in two seconds; one per two intervals would be four.
    assert!(sent >= 6, "keepalives in 2 s: {sent}");
    assert!(!connection.is_closed());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_keepalive_an_idle_connection_is_silent() {
    let (sent, alive) = idle_upstream_bytes(None).await;
    assert_eq!(sent, 0);
    assert!(alive);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unanswered_keepalives_drop_the_connection() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let proxy = FreezeProxy::start(server.addr()).await;
    let mut spec = spec(proxy.addr(), Some("alice"), password_plan(&password));
    spec.keepalive = Some(Duration::from_millis(300));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    proxy.freeze();
    let started = Instant::now();
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(
        output.end,
        Some(SessionEnd::Dropped(FailureKind::Timeout)),
        "{output:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(600),
        "three intervals: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert!(connection.is_closed());
    server.shutdown().await;
}

fn exec_config(password: &str) -> ServerConfig {
    let mut config = ServerConfig::new(vec![password_user("alice", password)]);
    config.exec = Some(ExecAnswer {
        stdout: "Linux\nID=debian\nVERSION_ID=12\n".to_owned(),
        exit_status: 0,
    });
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_collects_stdout_and_the_exit_status() {
    let password = random_secret("pw");
    let (server, connection) = server_and_connection(exec_config(&password), &password).await;
    let output = connection
        .exec(
            "uname -s; cat /etc/os-release",
            Duration::from_secs(5),
            4096,
        )
        .await
        .expect("exec");
    assert_eq!(output.stdout, b"Linux\nID=debian\nVERSION_ID=12\n");
    assert_eq!(output.exit_status, Some(0));
    assert!(server.events().contains(&ServerEvent::Exec {
        command: "uname -s; cat /etc/os-release".to_owned()
    }));

    let capped = connection
        .exec("uname -s", Duration::from_secs(5), 5)
        .await
        .expect("exec");
    assert_eq!(capped.stdout, b"Linux");
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_never_touches_the_shell() {
    let password = random_secret("pw");
    let (server, connection) = server_and_connection(exec_config(&password), &password).await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    let output = connection
        .exec("uname -s", Duration::from_secs(5), 4096)
        .await
        .expect("exec");
    assert!(output.stdout.starts_with(b"Linux"));
    shell
        .writer()
        .write(b"echo after-exec\r")
        .await
        .expect("write");
    let text = read_until(&mut shell, "\r\nafter-exec\r\n", SHORT).await;
    assert!(
        !text.contains("ID=debian"),
        "exec output must not reach the shell"
    );
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_exec_is_a_channel_failure() {
    let (server, connection) = simple().await;
    let error = connection
        .exec("uname -s", Duration::from_secs(5), 4096)
        .await
        .expect_err("the server refuses exec");
    assert_eq!(error.kind, FailureKind::ChannelFailed);
    assert!(!connection.is_closed());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_is_bounded_by_its_timeout() {
    let password = random_secret("pw");
    let server = start_with(exec_config(&password)).await;
    let proxy = FreezeProxy::start(server.addr()).await;
    let connection = connect(
        spec(proxy.addr(), Some("alice"), password_plan(&password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    proxy.freeze();
    let started = Instant::now();
    let error = connection
        .exec("uname -s", Duration::from_millis(500), 4096)
        .await
        .expect_err("no answer on a frozen link");
    assert_eq!(error.kind, FailureKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(connection);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_is_echoed_byte_for_byte_including_utf8() {
    let (server, connection) = simple().await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    let writer = shell.writer();
    writer
        .write("echo grüße-日本\r".as_bytes())
        .await
        .expect("write");
    read_until(&mut shell, "\r\ngrüße-日本\r\n", SHORT).await;
    // Large output arrives completely and in order.
    writer.write(b"lines 2000\r").await.expect("write");
    read_until(&mut shell, "line 2000\r\n", SHORT).await;
    writer.close().await;
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_after_the_end_fail_instead_of_waiting() {
    let (server, connection) = simple().await;
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    let writer = shell.writer();
    writer.write(b"exit 3\r").await.expect("write");
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(output.end, Some(SessionEnd::ExitStatus(3)));
    let result = tokio::time::timeout(Duration::from_secs(2), writer.write(b"late input")).await;
    let error = result.expect("does not hang").expect_err("the shell ended");
    assert_eq!(error.kind, FailureKind::ChannelFailed);
    // Resizes of an ended shell are harmless.
    let _ = writer.resize(90, 30).await;
    assert!(!connection.is_closed());
    connection.disconnect().await;
    server.shutdown().await;
}
